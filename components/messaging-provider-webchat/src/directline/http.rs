use base64::{Engine as _, engine::general_purpose};
use chrono::Utc;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use urlencoding::{decode, encode};
use uuid::Uuid;

use greentic_types::messaging::universal_dto::{Header, HttpInV1, HttpOutV1};

use super::caller::caller_header;
use super::jwt::{DirectLineContext, TTL_SECONDS, issue_token, reissue_token, verify_token};
use super::oidc::{OidcError, verify_access_token};
use super::state::{ConversationState, StoredActivity, conversation_key, sanitize_team};
use super::store::{JwksFetcher, NoJwksFetcher, RateLimitState, SecretStore, StateStore};

const DIRECTLINE_PREFIX: &str = "/v3/directline";
const JSON_CONTENT_TYPE: &str = "application/json";
const TOKEN_SECRET_KEY: &str = "jwt_signing_key";
const RATE_LIMIT_WINDOW_SECONDS_DEFAULT: i64 = 60;
const RATE_LIMIT_REQUESTS_DEFAULT: u32 = 60;
const MAX_ATTACHMENT_BYTES: usize = 512 * 1024;
const MAX_CHANNEL_DATA_BYTES: usize = 64 * 1024;
const FLOW_HINT_HEADER: &str = "X-Greentic-Flow";
const FLOW_HINT_MAX_LEN: usize = 256;
const ALLOWED_ATTACHMENT_TYPES: &[&str] = &[
    "text/plain",
    "application/json",
    "image/png",
    "image/jpeg",
    "image/gif",
    "application/vnd.microsoft.card.adaptive",
    "application/vnd.microsoft.card.hero",
    "application/vnd.microsoft.card.thumbnail",
];

pub fn handle_directline_request<S, SE>(
    request: &HttpInV1,
    state_store: &mut S,
    secrets: &SE,
) -> HttpOutV1
where
    S: StateStore,
    SE: SecretStore,
{
    handle_directline_request_with_jwks(request, state_store, secrets, &NoJwksFetcher)
}

pub fn handle_directline_request_with_jwks<S, SE, J>(
    request: &HttpInV1,
    state_store: &mut S,
    secrets: &SE,
    jwks: &J,
) -> HttpOutV1
where
    S: StateStore,
    SE: SecretStore,
    J: JwksFetcher,
{
    if !request.path.starts_with(DIRECTLINE_PREFIX) {
        return respond_not_found("missing directline prefix");
    }

    // Handle CORS preflight requests
    if method_is(request, "OPTIONS") {
        return respond_cors_preflight();
    }

    let segments = request
        .path
        .trim_start_matches('/')
        .split('/')
        .collect::<Vec<_>>();

    match segments.as_slice() {
        ["v3", "directline", "tokens", "generate"] if method_is(request, "POST") => {
            handle_tokens(request, state_store, secrets, jwks)
        }
        ["v3", "directline", "tokens", "generate"] => method_not_allowed(),
        ["v3", "directline", "tokens", "refresh"] if method_is(request, "POST") => {
            handle_refresh_token(request, state_store, secrets)
        }
        ["v3", "directline", "tokens", "refresh"] => method_not_allowed(),
        ["v3", "directline", "conversations"] if method_is(request, "POST") => {
            handle_conversations(request, state_store, secrets)
        }
        ["v3", "directline", "conversations"] => method_not_allowed(),
        ["v3", "directline", "conversations", conv_id, "activities"] => {
            match request.method.as_str() {
                m if m.eq_ignore_ascii_case("POST") => {
                    handle_post_activities(request, state_store, secrets, conv_id)
                }
                m if m.eq_ignore_ascii_case("GET") => {
                    handle_get_activities(request, state_store, secrets, conv_id)
                }
                _ => method_not_allowed(),
            }
        }
        ["v3", "directline", "conversations", conv_id] if method_is(request, "GET") => {
            handle_reconnect_conversation(request, state_store, secrets, conv_id)
        }
        ["v3", "directline", "conversations", _conv_id] => method_not_allowed(),
        ["v3", "directline", "conversations", _conv_id, "stream"] => respond_not_implemented(),
        _ => respond_not_found("unknown directline endpoint"),
    }
}

fn handle_tokens<S, SE, J>(
    request: &HttpInV1,
    state_store: &mut S,
    secrets: &SE,
    jwks: &J,
) -> HttpOutV1
where
    S: StateStore,
    SE: SecretStore,
    J: JwksFetcher,
{
    let ctx = parse_context(request.query.as_deref());
    let body = match decode_json_body(request) {
        Ok(payload) => payload,
        Err(resp) => return resp,
    };
    let subject = determine_rate_limit_subject(request, &body);
    let now = Utc::now().timestamp();
    let cfg = RateLimitConfig::from_request(request);
    let rate_key = rate_limit_key(&ctx, &subject);
    if let Err(resp) = enforce_rate_limit(state_store, &rate_key, now, &cfg) {
        return resp;
    }

    let (token_subject, verified) =
        match determine_verified_identity(request, state_store, jwks, now) {
            Ok(Some(identity)) => (identity, true),
            Ok(None) => {
                // Reject a client-supplied id shaped like an issuer subject
                // (`{did_web}:users:{user_id}`) — otherwise an anonymous
                // caller can self-declare exactly the id a verified bearer
                // would have produced, and the two become indistinguishable
                // downstream. Guest ids are UUIDs and never contain a colon.
                if let RateLimitSubject::User(ref id) = subject
                    && id.contains(":users:")
                {
                    return respond_error(
                        400,
                        "invalid_user_id",
                        "client-supplied user id must not use the issuer-subject shape",
                    );
                }
                (subject.token_subject().to_string(), false)
            }
            Err(resp) => return resp,
        };

    let signing_key = match load_signing_key(request, secrets) {
        Ok(key) => key,
        Err(resp) => return resp,
    };

    match issue_token(&signing_key, ctx.clone(), &token_subject, None, verified) {
        Ok((token, _exp)) => respond_json(
            200,
            json!({
                "token": token,
                "expires_in": TTL_SECONDS,
            }),
        ),
        Err(err) => respond_error(
            500,
            "token_issue_failed",
            format!("failed to mint token: {err:?}"),
        ),
    }
}

/// Resolves the subject of an OIDC bearer, if one was presented.
///
/// `Ok(None)` means no bearer was presented and the caller should fall back
/// to the anonymous/self-declared subject. Any bearer that fails to verify —
/// including a tenant with no `oidc_issuer` configured to check it against —
/// returns `Err` (401) rather than silently downgrading to anonymous.
fn determine_verified_identity<S, J>(
    request: &HttpInV1,
    state_store: &mut S,
    jwks: &J,
    now: i64,
) -> Result<Option<String>, HttpOutV1>
where
    S: StateStore,
    J: JwksFetcher,
{
    let bearer = extract_bearer(&request.headers);
    let Some(bearer) = bearer else {
        return Ok(None);
    };

    let Some(issuer) = config_str(request, "oidc_issuer") else {
        return Err(respond_error(
            401,
            "unauthorized",
            "bearer provided but oidc verification is not configured for this tenant",
        ));
    };
    // An http:// issuer would fetch signing keys in plaintext, letting a
    // network attacker substitute a JWKS and mint tokens that verify.
    if !issuer.starts_with("https://") {
        return Err(respond_error(
            401,
            "unauthorized",
            "oidc_issuer must use https",
        ));
    }
    let audience =
        config_str(request, "oidc_audience").unwrap_or_else(|| "webchat-gui".to_string());
    let required_scope = config_str(request, "oidc_required_scope")
        .unwrap_or_else(|| "greentic.webchat".to_string());

    let jwks_url = format!("{}/jwks.json", issuer.trim_end_matches('/'));
    let jwks_doc = match load_jwks(state_store, jwks, &jwks_url, now) {
        Ok(doc) => doc,
        Err(err) => {
            return Err(respond_error(
                401,
                "unauthorized",
                format!("jwks unavailable: {err}"),
            ));
        }
    };

    match verify_access_token(&bearer, &jwks_doc, &issuer, &audience, &required_scope, now) {
        Ok(identity) if identity.sub.is_empty() => Err(respond_error(
            401,
            "unauthorized",
            "access token rejected: empty subject",
        )),
        Ok(identity) => Ok(Some(identity.sub)),
        // The cached JWKS may be stale after the issuer rotated its signing
        // key. Refetch once, bypassing the cache, and retry verification —
        // bounded to a single retry so a persistently unknown kid can't turn
        // into a fetch amplifier, and gated by a per-issuer cooldown so a
        // flood of cheap forged tokens can't force an outbound fetch per
        // request (see `jwks_refetch_allowed`).
        Err(OidcError::UnknownKey) => {
            if !jwks_refetch_allowed(state_store, &jwks_url, now) {
                return Err(respond_error(
                    401,
                    "unauthorized",
                    format!("access token rejected: {:?}", OidcError::UnknownKey),
                ));
            }
            let fresh_doc = match refetch_jwks(state_store, jwks, &jwks_url, now) {
                Ok(doc) => doc,
                Err(err) => {
                    return Err(respond_error(
                        401,
                        "unauthorized",
                        format!("jwks unavailable: {err}"),
                    ));
                }
            };
            match verify_access_token(
                &bearer,
                &fresh_doc,
                &issuer,
                &audience,
                &required_scope,
                now,
            ) {
                Ok(identity) if identity.sub.is_empty() => Err(respond_error(
                    401,
                    "unauthorized",
                    "access token rejected: empty subject",
                )),
                Ok(identity) => Ok(Some(identity.sub)),
                Err(err) => Err(respond_error(
                    401,
                    "unauthorized",
                    format!("access token rejected: {err:?}"),
                )),
            }
        }
        Err(err) => Err(respond_error(
            401,
            "unauthorized",
            format!("access token rejected: {err:?}"),
        )),
    }
}

fn handle_conversations<S, SE>(request: &HttpInV1, state_store: &mut S, secrets: &SE) -> HttpOutV1
where
    S: StateStore,
    SE: SecretStore,
{
    let authorization = match extract_bearer(request.headers.as_slice()) {
        Some(header) => header,
        None => return respond_unauthorized("missing Authorization header"),
    };
    let signing_key = match load_signing_key(request, secrets) {
        Ok(key) => key,
        Err(resp) => return resp,
    };
    let claims = match verify_token(&signing_key, &authorization) {
        Ok(claims) => claims,
        Err(err) => return respond_unauthorized(&format!("invalid token: {err:?}")),
    };

    if claims.conv.is_some() {
        return respond_forbidden("token already bound to a conversation");
    }

    let ctx = claims.ctx.clone();
    let conversation_id = Uuid::new_v4().to_string();
    let key = conversation_key(&ctx, &conversation_id);
    let flow_hint = extract_flow_hint(&request.headers);
    let mut conversation = ConversationState::new(ctx.clone());
    conversation.flow_binding = flow_hint.clone();

    if let Err(resp) = write_conversation_state(state_store, &key, &conversation) {
        return resp;
    }

    let (token, _exp) = match reissue_token(&signing_key, &claims, Some(conversation_id.clone())) {
        Ok(pair) => pair,
        Err(err) => {
            return respond_error(
                500,
                "token_issue_failed",
                format!("failed to mint conversation token: {err:?}"),
            );
        }
    };

    // Include context in headers so ingest_http can extract env/tenant for autoStart envelope
    let mut headers = json_headers();
    headers.push(Header {
        name: "X-Greentic-Env".to_string(),
        value: ctx.env.clone(),
    });
    headers.push(Header {
        name: "X-Greentic-Tenant".to_string(),
        value: ctx.tenant.clone(),
    });
    headers.push(Header {
        name: "X-Greentic-User".to_string(),
        value: claims.sub.clone(),
    });
    headers.push(Header {
        name: "X-Greentic-User-Verified".to_string(),
        value: claims.verified.to_string(),
    });
    // The verified caller block for `extensions.caller`; consumed and removed
    // by ingest before the response leaves the provider (see `caller`).
    headers.push(caller_header(&claims));
    headers.push(Header {
        name: "X-Greentic-ConversationId".to_string(),
        value: conversation_id.clone(),
    });
    if let Some(ref flow) = flow_hint {
        headers.push(Header {
            name: FLOW_HINT_HEADER.to_string(),
            value: flow.clone(),
        });
    }

    let stream_url = build_stream_url(&ctx.tenant, &conversation_id, &token);
    respond_json_with_headers(
        201,
        json!({
            "conversationId": conversation_id,
            "token": token,
            "expires_in": TTL_SECONDS,
            "streamUrl": stream_url,
        }),
        headers,
    )
}

fn handle_refresh_token<S, SE>(request: &HttpInV1, state_store: &mut S, secrets: &SE) -> HttpOutV1
where
    S: StateStore,
    SE: SecretStore,
{
    let authorization = match extract_bearer(request.headers.as_slice()) {
        Some(header) => header,
        None => return respond_unauthorized("missing Authorization header"),
    };
    let signing_key = match load_signing_key(request, secrets) {
        Ok(key) => key,
        Err(resp) => return resp,
    };
    let claims = match verify_token(&signing_key, &authorization) {
        Ok(claims) => claims,
        Err(err) => return respond_unauthorized(&format!("invalid token: {err:?}")),
    };

    if let Some(conversation_id) = claims.conv.as_deref() {
        let conv_key = conversation_key(&claims.ctx, conversation_id);
        let conversation = match load_conversation_state(state_store, &conv_key) {
            Ok(state) => state,
            Err(resp) => return resp,
        };

        if conversation.ctx != claims.ctx {
            return respond_forbidden("token context mismatch");
        }
    }

    let (token, _exp) = match reissue_token(&signing_key, &claims, claims.conv.clone()) {
        Ok(pair) => pair,
        Err(err) => {
            return respond_error(
                500,
                "token_issue_failed",
                format!("failed to mint refresh token: {err:?}"),
            );
        }
    };

    respond_json(
        200,
        json!({
            "conversationId": claims.conv,
            "token": token,
            "expires_in": TTL_SECONDS,
        }),
    )
}

/// Handle GET /v3/directline/conversations/{conversationId} - reconnect to existing conversation.
/// Returns conversation info with a refreshed token if the conversation exists and token is valid.
fn handle_reconnect_conversation<S, SE>(
    request: &HttpInV1,
    state_store: &mut S,
    secrets: &SE,
    conversation_id: &str,
) -> HttpOutV1
where
    S: StateStore,
    SE: SecretStore,
{
    let authorization = match extract_bearer(request.headers.as_slice()) {
        Some(header) => header,
        None => return respond_unauthorized("missing Authorization header"),
    };
    let signing_key = match load_signing_key(request, secrets) {
        Ok(key) => key,
        Err(resp) => return resp,
    };
    let claims = match verify_token(&signing_key, &authorization) {
        Ok(claims) => claims,
        Err(err) => return respond_unauthorized(&format!("invalid token: {err:?}")),
    };

    // Token must be bound to this conversation (or unbound for first reconnect)
    if let Some(ref bound_conv) = claims.conv
        && bound_conv != conversation_id
    {
        return respond_forbidden("token bound to different conversation");
    }

    let ctx = claims.ctx.clone();
    let conv_key = conversation_key(&ctx, conversation_id);

    // Verify conversation exists
    match load_conversation_state(state_store, &conv_key) {
        Ok(conversation) => {
            if conversation.ctx != ctx {
                return respond_forbidden("token context mismatch");
            }
        }
        Err(resp) => return resp,
    };

    // Issue a new token bound to this conversation, carrying the caller's
    // verified claims (see `reissue_token`).
    let tenant_for_stream = ctx.tenant.clone();
    let (token, _exp) =
        match reissue_token(&signing_key, &claims, Some(conversation_id.to_string())) {
            Ok(pair) => pair,
            Err(err) => {
                return respond_error(
                    500,
                    "token_issue_failed",
                    format!("failed to mint reconnect token: {err:?}"),
                );
            }
        };

    let stream_url = build_stream_url(&tenant_for_stream, conversation_id, &token);
    respond_json(
        200,
        json!({
            "conversationId": conversation_id,
            "token": token,
            "expires_in": TTL_SECONDS,
            "streamUrl": stream_url,
        }),
    )
}

fn handle_post_activities<S, SE>(
    request: &HttpInV1,
    state_store: &mut S,
    secrets: &SE,
    conversation_id: &str,
) -> HttpOutV1
where
    S: StateStore,
    SE: SecretStore,
{
    let authorization = match extract_bearer(request.headers.as_slice()) {
        Some(token) => token,
        None => return respond_unauthorized("missing Authorization header"),
    };
    let signing_key = match load_signing_key(request, secrets) {
        Ok(key) => key,
        Err(resp) => return resp,
    };
    let claims = match verify_token(&signing_key, &authorization) {
        Ok(claims) => claims,
        Err(err) => return respond_unauthorized(&format!("invalid token: {err:?}")),
    };

    if claims.conv.as_deref() != Some(conversation_id) {
        return respond_forbidden("token bound to different conversation");
    }

    let conv_key = conversation_key(&claims.ctx, conversation_id);
    let mut conversation = match load_conversation_state(state_store, &conv_key) {
        Ok(state) => state,
        Err(resp) => return resp,
    };

    if conversation.ctx != claims.ctx {
        return respond_forbidden("token context mismatch");
    }

    let watermark = conversation.bump_watermark();
    let body = match decode_json_body(request) {
        Ok(value) => value,
        Err(resp) => return resp,
    };

    if let Err(resp) = validate_attachments(&body) {
        return resp;
    }

    if let Err(resp) = validate_channel_data(&body) {
        return resp;
    }

    let activity = StoredActivity {
        id: Uuid::new_v4().to_string(),
        type_: body
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("message")
            .to_string(),
        text: body
            .get("text")
            .and_then(|value| value.as_str())
            .map(|s| s.to_string()),
        from: body
            .get("from")
            .and_then(|from| from.get("id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        timestamp: Utc::now().timestamp_millis(),
        watermark,
        raw: body.clone(),
    };

    conversation.activities.push(activity.clone());

    if let Err(resp) = write_conversation_state(state_store, &conv_key, &conversation) {
        return resp;
    }

    // Include context in headers so ingest_http can extract env/tenant for envelope routing
    let mut headers = json_headers();
    headers.push(Header {
        name: "X-Greentic-Env".to_string(),
        value: claims.ctx.env.clone(),
    });
    headers.push(Header {
        name: "X-Greentic-Tenant".to_string(),
        value: claims.ctx.tenant.clone(),
    });
    headers.push(Header {
        name: "X-Greentic-User".to_string(),
        value: claims.sub.clone(),
    });
    headers.push(Header {
        name: "X-Greentic-User-Verified".to_string(),
        value: claims.verified.to_string(),
    });
    // The verified caller block for `extensions.caller`; consumed and removed
    // by ingest before the response leaves the provider (see `caller`).
    headers.push(caller_header(&claims));
    if let Some(ref flow) = conversation.flow_binding {
        headers.push(Header {
            name: FLOW_HINT_HEADER.to_string(),
            value: flow.clone(),
        });
    }

    // Emit `_greentic` metadata so the host can fire WebSocket push notifications
    // for live activity streams. Underscore-prefixed keys are ignored by DirectLine
    // clients, making this safe to add to the response body.
    let body_value = json!({
        "id": activity.id,
        "_greentic": {
            "watermark_bumped": watermark,
            "conversation_id": conversation_id,
            "tenant": claims.ctx.tenant,
        },
    });
    respond_json_with_headers(201, body_value, headers)
}

fn handle_get_activities<S, SE>(
    request: &HttpInV1,
    state_store: &mut S,
    secrets: &SE,
    conversation_id: &str,
) -> HttpOutV1
where
    S: StateStore,
    SE: SecretStore,
{
    let authorization = match extract_bearer(request.headers.as_slice()) {
        Some(token) => token,
        None => return respond_unauthorized("missing Authorization header"),
    };
    let signing_key = match load_signing_key(request, secrets) {
        Ok(key) => key,
        Err(resp) => return resp,
    };
    let claims = match verify_token(&signing_key, &authorization) {
        Ok(claims) => claims,
        Err(err) => return respond_unauthorized(&format!("invalid token: {err:?}")),
    };

    if claims.conv.as_deref() != Some(conversation_id) {
        return respond_forbidden("token bound to different conversation");
    }

    let conv_key = conversation_key(&claims.ctx, conversation_id);
    let conversation = match load_conversation_state(state_store, &conv_key) {
        Ok(state) => state,
        Err(resp) => return resp,
    };

    if conversation.ctx != claims.ctx {
        return respond_forbidden("token context mismatch");
    }

    let watermark = match parse_watermark(request.query.as_deref()) {
        Ok(value) => value,
        Err(resp) => return resp,
    };

    let activities = conversation
        .activities
        .iter()
        .filter(|activity| match watermark {
            Some(watermark) => activity.watermark >= watermark,
            None => true,
        })
        .map(activity_to_value)
        .collect::<Vec<_>>();

    respond_json(
        200,
        json!({
            "activities": activities,
            "watermark": conversation.next_watermark.to_string(),
        }),
    )
}

fn enforce_rate_limit<S: StateStore>(
    store: &mut S,
    key: &str,
    now: i64,
    cfg: &RateLimitConfig,
) -> Result<(), HttpOutV1> {
    let mut state = match read_rate_limit_state(store, key) {
        Ok(Some(state)) => state,
        Ok(None) => RateLimitState::new(now),
        Err(resp) => return Err(resp),
    };

    if state.bump(now, cfg.window_seconds, cfg.requests).is_err() {
        let retry_after = (cfg.window_seconds - (now - state.window_start)).max(1);
        return Err(respond_rate_limited(retry_after));
    }

    let bytes = match serde_json::to_vec(&state) {
        Ok(bytes) => bytes,
        Err(err) => return Err(respond_error(500, "state_serialize", err.to_string())),
    };

    store
        .write(key, &bytes)
        .map_err(|err| respond_error(500, "state_write", err))
}

fn read_rate_limit_state<S: StateStore>(
    store: &mut S,
    key: &str,
) -> Result<Option<RateLimitState>, HttpOutV1> {
    match store.read(key) {
        Ok(Some(bytes)) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|err| respond_error(500, "state_parse", err.to_string())),
        Ok(None) => Ok(None),
        Err(err) => Err(respond_error(500, "state_read", err)),
    }
}

fn write_conversation_state<S: StateStore>(
    store: &mut S,
    key: &str,
    state: &ConversationState,
) -> Result<(), HttpOutV1> {
    let bytes = serde_json::to_vec(state)
        .map_err(|err| respond_error(500, "state_serialize", err.to_string()))?;
    store
        .write(key, &bytes)
        .map_err(|err| respond_error(500, "state_write", err))
}

fn load_conversation_state<S: StateStore>(
    store: &mut S,
    key: &str,
) -> Result<ConversationState, HttpOutV1> {
    match store.read(key) {
        Ok(Some(bytes)) => serde_json::from_slice(&bytes)
            .map_err(|err| respond_error(500, "state_parse", err.to_string())),
        Ok(None) => Err(respond_not_found("conversation not found")),
        Err(err) => Err(respond_error(500, "state_read", err)),
    }
}

fn activity_to_value(activity: &StoredActivity) -> Value {
    let mut map = match activity.raw.clone() {
        Value::Object(map) => map,
        other => {
            let mut map = Map::new();
            map.insert("data".to_string(), other);
            map
        }
    };
    map.insert("id".to_string(), Value::String(activity.id.clone()));
    map.insert("type".to_string(), Value::String(activity.type_.clone()));
    // Format timestamp as ISO 8601 (WebChat SDK expects this, not raw millis).
    let ts_iso = chrono::DateTime::from_timestamp_millis(activity.timestamp)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| activity.timestamp.to_string());
    map.insert("timestamp".to_string(), Value::String(ts_iso));
    map.insert(
        "watermark".to_string(),
        Value::String(activity.watermark.to_string()),
    );
    if let Some(text) = &activity.text {
        map.insert("text".to_string(), Value::String(text.clone()));
    }
    if !map.contains_key("from")
        && let Some(from) = &activity.from
    {
        let mut from_map = Map::new();
        from_map.insert("id".to_string(), Value::String(from.clone()));
        map.insert("from".to_string(), Value::Object(from_map));
    }
    Value::Object(map)
}

fn validate_attachments(body: &Value) -> Result<(), HttpOutV1> {
    let attachments = match body.get("attachments") {
        Some(Value::Array(items)) => items,
        _ => return Ok(()),
    };

    for attachment in attachments {
        let content_type = attachment
            .get("contentType")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        if !ALLOWED_ATTACHMENT_TYPES.contains(&content_type) {
            return Err(respond_bad_request(&format!(
                "unsupported content type: {content_type}"
            )));
        }

        if let Some(content) = attachment.get("content")
            && let Some(text) = content.as_str()
            && text.len() > MAX_ATTACHMENT_BYTES
        {
            return Err(respond_bad_request("attachment too large"));
        }
    }

    Ok(())
}

// channelData is stored verbatim on every activity, so an uncapped payload
// grows conversation state without bound.
fn validate_channel_data(body: &Value) -> Result<(), HttpOutV1> {
    let channel_data = match body.get("channelData") {
        Some(value) if !value.is_null() => value,
        _ => return Ok(()),
    };

    match serde_json::to_string(channel_data) {
        Ok(encoded) if encoded.len() > MAX_CHANNEL_DATA_BYTES => {
            Err(respond_bad_request("channelData too large"))
        }
        Ok(_) => Ok(()),
        Err(_) => Err(respond_bad_request("channelData is not serialisable")),
    }
}

fn parse_watermark(query: Option<&str>) -> Result<Option<u64>, HttpOutV1> {
    let params = parse_query(query);
    if let Some(value) = params.get("watermark") {
        if value.is_empty() {
            return Ok(None);
        }
        value
            .parse::<u64>()
            .map(Some)
            .map_err(|_| respond_bad_request("watermark must be a number"))
    } else {
        Ok(None)
    }
}

fn rate_limit_key(ctx: &DirectLineContext, subject: &RateLimitSubject) -> String {
    format!(
        "webchat:rate:tokens:{}:{}:{}:{}",
        ctx.env,
        ctx.tenant,
        sanitize_team(ctx.team.as_deref()),
        subject.bucket_key()
    )
}

/// Per-route rate limit configuration. Operators can override defaults via
/// the provider config envelope: `rate_limit_requests` (per-window cap) and
/// `rate_limit_window_seconds` (window length). Both fall back to sensible
/// defaults that target ~60 token mints / minute / subject — enough for
/// normal page reload + retry burst, while still bounding abuse.
#[derive(Clone, Copy, Debug)]
struct RateLimitConfig {
    window_seconds: i64,
    requests: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            window_seconds: RATE_LIMIT_WINDOW_SECONDS_DEFAULT,
            requests: RATE_LIMIT_REQUESTS_DEFAULT,
        }
    }
}

impl RateLimitConfig {
    fn from_request(request: &HttpInV1) -> Self {
        let mut cfg = Self::default();
        let Some(config) = request.config.as_ref() else {
            return cfg;
        };
        if let Some(req) = config.get("rate_limit_requests").and_then(Value::as_u64)
            && req > 0
            && req <= u32::MAX as u64
        {
            cfg.requests = req as u32;
        }
        if let Some(win) = config
            .get("rate_limit_window_seconds")
            .and_then(Value::as_i64)
            && win > 0
        {
            cfg.window_seconds = win;
        }
        cfg
    }
}

/// Bucket subject for rate limiting.
///
/// Authenticated users get their own per-user bucket. Anonymous requests
/// are bucketed by hashed client IP (read from forwarding headers) so that
/// 1000 unauthenticated visitors don't all share the single `anonymous`
/// bucket. If neither id nor IP is available, a true anonymous bucket is
/// used as last resort — operators behind misconfigured reverse proxies
/// will see the legacy shared-bucket behaviour and should fix proxy
/// `X-Forwarded-For` propagation.
#[derive(Clone, Debug, PartialEq)]
enum RateLimitSubject {
    User(String),
    Ip(String),
    Anonymous,
}

impl RateLimitSubject {
    fn bucket_key(&self) -> String {
        match self {
            Self::User(id) => format!("u:{id}"),
            Self::Ip(hash) => format!("ip:{hash}"),
            Self::Anonymous => "anonymous".to_string(),
        }
    }

    fn token_subject(&self) -> &str {
        match self {
            Self::User(id) => id.as_str(),
            Self::Ip(hash) => hash.as_str(),
            Self::Anonymous => "anonymous",
        }
    }
}

fn determine_rate_limit_subject(request: &HttpInV1, body: &Value) -> RateLimitSubject {
    if let Some(id) = body
        .get("user")
        .and_then(|v| v.get("id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return RateLimitSubject::User(id.to_string());
    }
    if let Some(ip) = extract_client_ip(&request.headers) {
        return RateLimitSubject::Ip(hash_client_id(&ip));
    }
    RateLimitSubject::Anonymous
}

/// Extract the originating client IP from common forwarding headers.
///
/// Priority: `True-Client-IP` (Cloudflare/Akamai) > `X-Real-IP` (nginx) >
/// first hop of `X-Forwarded-For` (RFC 7239 / generic). Returns the raw
/// string; callers should hash before persisting or logging.
fn extract_client_ip(headers: &[Header]) -> Option<String> {
    for header_name in ["true-client-ip", "x-real-ip", "x-forwarded-for"] {
        for header in headers {
            if !header.name.eq_ignore_ascii_case(header_name) {
                continue;
            }
            let first_hop = header.value.split(',').next().unwrap_or("").trim();
            if !first_hop.is_empty() {
                return Some(first_hop.to_string());
            }
        }
    }
    None
}

/// Hash a client identifier (typically an IP) to a short stable token used
/// for rate-limit bucketing. We never persist the raw IP — only the hash.
fn hash_client_id(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    let hash = hasher.finalize();
    let mut out = String::with_capacity(16);
    for byte in &hash[..8] {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

fn config_str(request: &HttpInV1, key: &str) -> Option<String> {
    let cfg = request.config.as_ref()?;
    let plain = cfg
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    if plain.is_some() {
        return plain;
    }
    // The host base64-encodes every config value into `<key>_b64` before it
    // reaches the component, so the plain key is absent in a real deployment.
    // Reading only that made oidc_issuer look unconfigured, and every verified
    // bearer was rejected with "oidc verification is not configured".
    decode_injected_config_str(cfg, key)
}

fn decode_injected_config_str(cfg: &Value, key: &str) -> Option<String> {
    let encoded = cfg.get(format!("{key}_b64"))?.as_str()?.trim();
    if encoded.is_empty() {
        return None;
    }
    let decoded = general_purpose::STANDARD
        .decode(encoded)
        .or_else(|_| general_purpose::URL_SAFE.decode(encoded))
        .or_else(|_| general_purpose::URL_SAFE_NO_PAD.decode(encoded))
        .ok()?;
    let text = String::from_utf8(decoded).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_owned())
}

const JWKS_CACHE_TTL_SECONDS: i64 = 15 * 60;

#[derive(serde::Serialize, serde::Deserialize)]
struct CachedJwks {
    document: String,
    fetched_at: i64,
}

// Cache key is the issuer-derived URL, never the bearer token or its hash —
// ECDSA signatures are malleable, so a token-keyed cache would be
// bypassable by a re-encoded twin.
fn jwks_cache_key(jwks_url: &str) -> String {
    format!("webchat:jwks:{jwks_url}")
}

/// Caches the fetched document under `jwks_url`. A cache write failure just
/// costs a refetch next time — failing the mint over it would turn a
/// degraded state store into a login outage.
fn store_jwks_cache<S: StateStore>(state_store: &mut S, jwks_url: &str, document: &str, now: i64) {
    let entry = CachedJwks {
        document: document.to_string(),
        fetched_at: now,
    };
    if let Ok(bytes) = serde_json::to_vec(&entry) {
        let _ = state_store.write(&jwks_cache_key(jwks_url), &bytes);
    }
}

/// Fetches an issuer's JWKS document, caching by issuer-derived URL so a
/// page reload doesn't cost an outbound round trip on every token mint.
fn load_jwks<S, J>(
    state_store: &mut S,
    jwks: &J,
    jwks_url: &str,
    now: i64,
) -> Result<String, String>
where
    S: StateStore,
    J: JwksFetcher,
{
    let cache_key = jwks_cache_key(jwks_url);
    if let Ok(Some(bytes)) = state_store.read(&cache_key)
        && let Ok(cached) = serde_json::from_slice::<CachedJwks>(&bytes)
        && now - cached.fetched_at < JWKS_CACHE_TTL_SECONDS
    {
        return Ok(cached.document);
    }
    let document = jwks.fetch(jwks_url)?;
    store_jwks_cache(state_store, jwks_url, &document, now);
    Ok(document)
}

/// Bypasses the cache entirely and overwrites it with a fresh fetch. Used
/// once, after a verification failure that looks like a rotated signing key
/// (`OidcError::UnknownKey`) — never called recursively, so it cannot become
/// a fetch amplifier.
fn refetch_jwks<S, J>(
    state_store: &mut S,
    jwks: &J,
    jwks_url: &str,
    now: i64,
) -> Result<String, String>
where
    S: StateStore,
    J: JwksFetcher,
{
    let document = jwks.fetch(jwks_url)?;
    store_jwks_cache(state_store, jwks_url, &document, now);
    Ok(document)
}

/// Minimum interval between `refetch_jwks` calls for the same issuer.
/// `verify_access_token` returns `UnknownKey` from the kid filter before it
/// checks the signature, `iss`, `aud`, `exp`, or scope — the cheapest
/// possible forged token reaches this arm. Without a cooldown, an attacker
/// could force one outbound HTTPS fetch to the issuer's `/jwks.json` per
/// request; the existing rate limiter doesn't stop this because it buckets
/// per subject/IP, both of which an attacker can rotate freely. The issuer,
/// not us, would be the amplification victim.
const JWKS_REFETCH_COOLDOWN_SECONDS: i64 = 60;

fn jwks_refetch_key(jwks_url: &str) -> String {
    format!("webchat:jwks:refetch:{jwks_url}")
}

#[derive(serde::Serialize, serde::Deserialize)]
struct JwksRefetchAttempt {
    attempted_at: i64,
}

/// Returns whether a bounded `refetch_jwks` call is allowed right now for
/// this issuer, and — if so — immediately records the attempt (before the
/// fetch itself happens) so a failing issuer can't be hammered by repeatedly
/// missing its own deadline either.
fn jwks_refetch_allowed<S: StateStore>(state_store: &mut S, jwks_url: &str, now: i64) -> bool {
    let key = jwks_refetch_key(jwks_url);
    let allowed = match state_store.read(&key) {
        // A read error is not evidence that no attempt was made — fail
        // closed, matching `read_rate_limit_state`/`load_conversation_state`'s
        // convention elsewhere in this file. A store an attacker can make
        // unhealthy must not become a way to switch this control off.
        Err(_) => false,
        Ok(None) => true,
        Ok(Some(bytes)) => match serde_json::from_slice::<JwksRefetchAttempt>(&bytes) {
            Ok(attempt) => now - attempt.attempted_at >= JWKS_REFETCH_COOLDOWN_SECONDS,
            // Corrupt data isn't a store failure — self-heal by allowing
            // once (the write below overwrites it) rather than blocking
            // rotation recovery forever.
            Err(_) => true,
        },
    };
    if !allowed {
        return false;
    }
    // Record-then-fetch: if the attempt can't be persisted, the fetch must
    // not happen either — a store that accepts reads but rejects writes
    // would otherwise be an uncapped amplifier again.
    let attempt = JwksRefetchAttempt { attempted_at: now };
    match serde_json::to_vec(&attempt) {
        Ok(bytes) => state_store.write(&key, &bytes).is_ok(),
        Err(_) => false,
    }
}

/// Load the JWT signing key from either injected config or secrets store.
/// Injected config takes priority (for provider_core_only mode where host pre-fetches secrets).
fn load_signing_key<SE: SecretStore>(
    request: &HttpInV1,
    secrets: &SE,
) -> Result<Vec<u8>, HttpOutV1> {
    // First, check for host-injected secret in config (for provider_core_only mode)
    if let Some(config) = &request.config {
        let config_key = format!("{TOKEN_SECRET_KEY}_b64");
        if let Some(b64_value) = config.get(&config_key).and_then(|v| v.as_str()) {
            return general_purpose::STANDARD
                .decode(b64_value)
                .map_err(|err| {
                    respond_error(
                        500,
                        "config_decode_error",
                        format!("failed to decode {config_key} from config: {err}"),
                    )
                })
                .and_then(|bytes| {
                    if bytes.is_empty() {
                        Err(respond_error(500, "invalid_secret", "signing key is empty"))
                    } else {
                        Ok(bytes)
                    }
                });
        }
    }

    // Fall back to secrets store
    match secrets.get(TOKEN_SECRET_KEY) {
        Ok(Some(bytes)) if !bytes.is_empty() => Ok(bytes),
        Ok(Some(_)) => Err(respond_error(500, "invalid_secret", "signing key is empty")),
        Ok(None) => Err(respond_error(
            500,
            "missing_secret",
            format!("secret {TOKEN_SECRET_KEY} not found"),
        )),
        Err(err) => Err(respond_error(500, "secret_error", err)),
    }
}

fn parse_context(query: Option<&str>) -> DirectLineContext {
    let params = parse_query(query);
    let env = params
        .get("env")
        .filter(|value| !value.is_empty())
        .cloned()
        .unwrap_or_else(|| "default".to_string());
    let tenant = params
        .get("tenant")
        .filter(|value| !value.is_empty())
        .cloned()
        .unwrap_or_else(|| "default".to_string());
    let team = params.get("team").and_then(|team| {
        let trimmed = team.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });

    DirectLineContext { env, tenant, team }
}

fn parse_query(query: Option<&str>) -> HashMap<String, String> {
    let mut map = HashMap::new();
    if let Some(query) = query {
        for pair in query.split('&') {
            if pair.is_empty() {
                continue;
            }
            let mut split = pair.splitn(2, '=');
            let key = split.next().unwrap_or_default();
            let value = split.next().unwrap_or_default();
            if let (Ok(key), Ok(value)) = (decode(key), decode(value)) {
                map.insert(key.into_owned(), value.into_owned());
            }
        }
    }
    map
}

fn decode_json_body(request: &HttpInV1) -> Result<Value, HttpOutV1> {
    if request.body_b64.trim().is_empty() {
        return Ok(Value::Null);
    }
    let bytes = match general_purpose::STANDARD.decode(&request.body_b64) {
        Ok(bytes) => bytes,
        Err(err) => {
            return Err(respond_bad_request(&format!(
                "invalid body encoding: {err}"
            )));
        }
    };
    serde_json::from_slice(&bytes)
        .map_err(|err| respond_bad_request(&format!("invalid json payload: {err}")))
}

fn extract_bearer(headers: &[Header]) -> Option<String> {
    headers
        .iter()
        .find(|header| header.name.eq_ignore_ascii_case("Authorization"))
        .and_then(|header| {
            let value = header.value.trim();
            let mut parts = value.splitn(2, ' ');
            let scheme = parts.next().unwrap_or_default();
            if scheme.eq_ignore_ascii_case("bearer") {
                Some(parts.next().unwrap_or_default().trim().to_string())
            } else {
                None
            }
        })
}

fn method_is(request: &HttpInV1, method: &str) -> bool {
    request.method.eq_ignore_ascii_case(method)
}

fn json_headers() -> Vec<Header> {
    vec![
        Header {
            name: "Content-Type".to_string(),
            value: JSON_CONTENT_TYPE.to_string(),
        },
        Header {
            name: "Access-Control-Allow-Origin".to_string(),
            value: "*".to_string(),
        },
        Header {
            name: "Access-Control-Allow-Headers".to_string(),
            value: "Authorization, Content-Type".to_string(),
        },
        Header {
            name: "Access-Control-Allow-Methods".to_string(),
            value: "GET, POST, OPTIONS".to_string(),
        },
    ]
}

fn respond_json(status: u16, payload: Value) -> HttpOutV1 {
    respond_json_with_headers(status, payload, json_headers())
}

fn respond_json_with_headers(status: u16, payload: Value, headers: Vec<Header>) -> HttpOutV1 {
    let body = serde_json::to_vec(&payload).unwrap_or_else(|_| b"{}".to_vec());
    HttpOutV1 {
        status,
        headers,
        body_b64: general_purpose::STANDARD.encode(&body),
        events: Vec::new(),
    }
}

fn respond_error(status: u16, error: &str, message: impl Into<String>) -> HttpOutV1 {
    respond_json(
        status,
        json!({
            "error": error,
            "message": message.into(),
        }),
    )
}

fn respond_rate_limited(retry_after_seconds: i64) -> HttpOutV1 {
    let mut headers = json_headers();
    headers.push(Header {
        name: "Retry-After".to_string(),
        value: retry_after_seconds.to_string(),
    });
    respond_json_with_headers(
        429,
        json!({
            "error": "rate_limited",
            "message": "token rate limit exceeded",
            "retry_after": retry_after_seconds,
        }),
        headers,
    )
}

fn respond_bad_request(message: &str) -> HttpOutV1 {
    respond_error(400, "bad_request", message)
}

fn respond_not_found(message: &str) -> HttpOutV1 {
    respond_error(404, "not_found", message)
}

fn method_not_allowed() -> HttpOutV1 {
    respond_error(
        405,
        "method_not_allowed",
        "method not allowed on this endpoint",
    )
}

fn respond_not_implemented() -> HttpOutV1 {
    respond_error(501, "not_implemented", "streaming not supported")
}

fn respond_unauthorized(message: &str) -> HttpOutV1 {
    respond_error(401, "unauthorized", message)
}

fn respond_forbidden(message: &str) -> HttpOutV1 {
    respond_error(403, "forbidden", message)
}

fn respond_cors_preflight() -> HttpOutV1 {
    HttpOutV1 {
        status: 204,
        headers: json_headers(),
        body_b64: String::new(),
        events: Vec::new(),
    }
}

/// Build the WebSocket `streamUrl` advertised to BotFramework-WebChat clients.
///
/// The URL must match the host's WS upgrade route, which is registered under
/// the tenant-scoped prefix `/v1/messaging/webchat/{tenant}/v3/directline/...`.
/// Token rides in the `t` query param because browser WebSocket APIs cannot
/// set custom headers.
///
/// `watermark=-1` instructs the server to replay all activities since the
/// conversation began (the host parses negative or missing values as 0).
fn build_stream_url(tenant: &str, conversation_id: &str, token: &str) -> String {
    format!(
        "{base}/v1/messaging/webchat/{tenant}/v3/directline/conversations/{conv_id}/stream?watermark=-1&t={token}",
        base = public_base_url_or_relative(),
        tenant = encode(tenant),
        conv_id = conversation_id,
        token = encode(token),
    )
}

/// Resolve the public base URL for the streamUrl.
///
/// WASM components cannot read host environment variables directly; supplying
/// an absolute URL would require a host import that does not yet exist. For
/// the MVP we return an empty string so the resulting `streamUrl` is a
/// site-relative path. Browsers (and the BotFramework WebChat SDK) resolve
/// such paths against the page origin and automatically upgrade `http(s)://`
/// to `ws(s)://` when opening the WebSocket.
fn public_base_url_or_relative() -> String {
    String::new()
}

/// Extract and validate the `X-Greentic-Flow` header from the request.
/// Returns `Some(flow_id)` only when the value is non-empty, within length
/// bounds, and free of control characters.
fn extract_flow_hint(headers: &[Header]) -> Option<String> {
    let raw = headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case(FLOW_HINT_HEADER))?
        .value
        .trim()
        .to_string();
    if raw.is_empty() || raw.len() > FLOW_HINT_MAX_LEN {
        return None;
    }
    if raw.chars().any(|c| c.is_control()) {
        return None;
    }
    Some(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose;

    #[test]
    fn config_str_reads_the_hosts_base64_injected_form() {
        // The host hands the component `<key>_b64`, never the plain key, so a
        // reader that only knows the plain form sees an unconfigured tenant.
        let request = HttpInV1 {
            method: "POST".to_string(),
            path: "/token".to_string(),
            query: None,
            headers: Vec::new(),
            body_b64: String::new(),
            route_hint: None,
            binding_id: None,
            config: Some(serde_json::json!({
                "oidc_issuer_b64": general_purpose::STANDARD.encode("https://id.example.com"),
                "oidc_audience": "plain-still-wins"
            })),
        };
        assert_eq!(
            config_str(&request, "oidc_issuer").as_deref(),
            Some("https://id.example.com")
        );
        assert_eq!(
            config_str(&request, "oidc_audience").as_deref(),
            Some("plain-still-wins")
        );
        assert_eq!(config_str(&request, "oidc_required_scope"), None);
    }
    use serde_json::json;
    use std::collections::HashMap;

    struct InMemoryStateStore {
        data: HashMap<String, Vec<u8>>,
    }

    impl InMemoryStateStore {
        fn new() -> Self {
            Self {
                data: HashMap::new(),
            }
        }
    }

    impl StateStore for InMemoryStateStore {
        fn read(&mut self, key: &str) -> Result<Option<Vec<u8>>, String> {
            Ok(self.data.get(key).cloned())
        }

        fn write(&mut self, key: &str, value: &[u8]) -> Result<(), String> {
            self.data.insert(key.to_string(), value.to_vec());
            Ok(())
        }
    }

    struct TestSecretStore {
        data: HashMap<String, Vec<u8>>,
    }

    impl TestSecretStore {
        fn new() -> Self {
            Self {
                data: HashMap::new(),
            }
        }

        fn insert(&mut self, key: &str, value: &[u8]) {
            self.data.insert(key.to_string(), value.to_vec());
        }
    }

    impl SecretStore for TestSecretStore {
        fn get(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
            Ok(self.data.get(key).cloned())
        }
    }

    fn build_request(
        method: &str,
        path: &str,
        query: Option<&str>,
        body: Option<&Value>,
        headers: Vec<Header>,
    ) -> Result<HttpInV1, String> {
        let body_b64 = match body {
            Some(payload) => general_purpose::STANDARD
                .encode(serde_json::to_vec(payload).map_err(|err| err.to_string())?),
            None => String::new(),
        };
        Ok(HttpInV1 {
            method: method.to_string(),
            path: path.to_string(),
            query: query.map(|value| value.to_string()),
            headers,
            body_b64,
            route_hint: None,
            binding_id: None,
            config: None,
        })
    }

    fn decode_body(response: &HttpOutV1) -> Result<Value, String> {
        let bytes = general_purpose::STANDARD
            .decode(&response.body_b64)
            .map_err(|err| err.to_string())?;
        serde_json::from_slice(&bytes).map_err(|err| err.to_string())
    }

    #[test]
    fn directline_polling_flow() -> Result<(), String> {
        let mut state = InMemoryStateStore::new();
        let mut secrets = TestSecretStore::new();
        secrets.insert(TOKEN_SECRET_KEY, b"test-secret");

        let token_request = build_request(
            "POST",
            "/v3/directline/tokens/generate",
            Some("env=default&tenant=default"),
            Some(&json!({"user": {"id": "alice"}})),
            vec![],
        )?;
        let token_response = handle_directline_request(&token_request, &mut state, &secrets);
        assert_eq!(token_response.status, 200);
        let token_body = decode_body(&token_response)?;
        let user_token = token_body["token"].as_str().ok_or("token returned")?;

        let conversation_request = build_request(
            "POST",
            "/v3/directline/conversations",
            None,
            None,
            vec![Header {
                name: "Authorization".into(),
                value: format!("Bearer {user_token}"),
            }],
        )?;
        let conversation_response =
            handle_directline_request(&conversation_request, &mut state, &secrets);
        assert_eq!(conversation_response.status, 201);
        let conversation_body = decode_body(&conversation_response)?;
        let conversation_id = conversation_body["conversationId"]
            .as_str()
            .ok_or("conversation id")?;
        let conv_token = conversation_body["token"]
            .as_str()
            .ok_or("conversation token")?;

        let reuse_response = handle_directline_request(
            &build_request(
                "POST",
                "/v3/directline/conversations",
                None,
                None,
                vec![Header {
                    name: "Authorization".into(),
                    value: format!("Bearer {conv_token}"),
                }],
            )?,
            &mut state,
            &secrets,
        );
        assert_eq!(reuse_response.status, 403);

        let activity = json!({
            "type": "message",
            "text": "hello",
            "from": {"id": "alice"},
        });
        let post_activity_response = handle_directline_request(
            &build_request(
                "POST",
                &format!("/v3/directline/conversations/{conversation_id}/activities"),
                None,
                Some(&activity),
                vec![Header {
                    name: "Authorization".into(),
                    value: format!("Bearer {conv_token}"),
                }],
            )?,
            &mut state,
            &secrets,
        );
        assert_eq!(post_activity_response.status, 201);
        let posted = decode_body(&post_activity_response)?;
        assert!(posted.get("id").is_some());

        let get_response = handle_directline_request(
            &build_request(
                "GET",
                &format!("/v3/directline/conversations/{conversation_id}/activities"),
                None,
                None,
                vec![Header {
                    name: "Authorization".into(),
                    value: format!("Bearer {conv_token}"),
                }],
            )?,
            &mut state,
            &secrets,
        );
        assert_eq!(get_response.status, 200);
        let get_body = decode_body(&get_response)?;
        let activities = get_body["activities"]
            .as_array()
            .ok_or("activities returned")?;
        assert_eq!(activities.len(), 1);
        assert_eq!(get_body["watermark"], Value::String("1".to_string()));

        let empty_response = handle_directline_request(
            &build_request(
                "GET",
                &format!("/v3/directline/conversations/{conversation_id}/activities"),
                Some("watermark=1"),
                None,
                vec![Header {
                    name: "Authorization".into(),
                    value: format!("Bearer {conv_token}"),
                }],
            )?,
            &mut state,
            &secrets,
        );
        assert_eq!(empty_response.status, 200);
        let empty_body = decode_body(&empty_response)?;
        assert!(
            empty_body["activities"]
                .as_array()
                .ok_or("activities returned")?
                .is_empty()
        );
        assert_eq!(empty_body["watermark"], Value::String("1".to_string()));

        let refresh_response = handle_directline_request(
            &build_request(
                "POST",
                "/v3/directline/tokens/refresh",
                None,
                None,
                vec![Header {
                    name: "Authorization".into(),
                    value: format!("Bearer {conv_token}"),
                }],
            )?,
            &mut state,
            &secrets,
        );
        assert_eq!(refresh_response.status, 200);
        let refresh_body = decode_body(&refresh_response)?;
        assert_eq!(
            refresh_body["conversationId"],
            Value::String(conversation_id.to_string())
        );
        assert!(refresh_body["token"].as_str().is_some());

        let wrong_conv_response = handle_directline_request(
            &build_request(
                "POST",
                "/v3/directline/conversations/other/activities",
                None,
                Some(&activity),
                vec![Header {
                    name: "Authorization".into(),
                    value: format!("Bearer {conv_token}"),
                }],
            )?,
            &mut state,
            &secrets,
        );
        assert_eq!(wrong_conv_response.status, 403);
        Ok(())
    }

    fn token_request_with_ip(client_ip: &str) -> Result<HttpInV1, String> {
        build_request(
            "POST",
            "/v3/directline/tokens/generate",
            Some("env=default&tenant=default"),
            None,
            vec![Header {
                name: "X-Forwarded-For".into(),
                value: client_ip.to_string(),
            }],
        )
    }

    fn header_value<'a>(response: &'a HttpOutV1, name: &str) -> Option<&'a str> {
        response
            .headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case(name))
            .map(|h| h.value.as_str())
    }

    #[test]
    fn anonymous_visitors_with_distinct_ips_get_independent_buckets() -> Result<(), String> {
        let mut state = InMemoryStateStore::new();
        let mut secrets = TestSecretStore::new();
        secrets.insert(TOKEN_SECRET_KEY, b"test-secret");

        // Send the default per-window cap from one IP — every call should
        // succeed because the bucket only fills for that IP.
        for _ in 0..RATE_LIMIT_REQUESTS_DEFAULT {
            let resp = handle_directline_request(
                &token_request_with_ip("203.0.113.10")?,
                &mut state,
                &secrets,
            );
            assert_eq!(resp.status, 200, "first IP should not be rate-limited");
        }

        // A different IP arriving immediately afterwards must still get a
        // fresh bucket — pre-fix this would have been blocked because
        // both anonymous visitors collapsed onto the same `anonymous`
        // bucket key.
        let other_ip_resp = handle_directline_request(
            &token_request_with_ip("198.51.100.20")?,
            &mut state,
            &secrets,
        );
        assert_eq!(
            other_ip_resp.status, 200,
            "distinct anonymous IP must not share rate-limit bucket"
        );
        Ok(())
    }

    #[test]
    fn xff_chain_uses_first_hop_for_bucket() -> Result<(), String> {
        let mut state = InMemoryStateStore::new();
        let mut secrets = TestSecretStore::new();
        secrets.insert(TOKEN_SECRET_KEY, b"test-secret");

        for _ in 0..RATE_LIMIT_REQUESTS_DEFAULT {
            assert_eq!(
                handle_directline_request(
                    &token_request_with_ip("203.0.113.10, 10.0.0.1")?,
                    &mut state,
                    &secrets,
                )
                .status,
                200
            );
        }

        // Same first hop, different proxy chain — must be the *same*
        // bucket because identity is decided by the leftmost hop only.
        let blocked = handle_directline_request(
            &token_request_with_ip("203.0.113.10, 10.0.0.99")?,
            &mut state,
            &secrets,
        );
        assert_eq!(blocked.status, 429);
        Ok(())
    }

    #[test]
    fn rate_limit_response_includes_retry_after_header() -> Result<(), String> {
        let mut state = InMemoryStateStore::new();
        let mut secrets = TestSecretStore::new();
        secrets.insert(TOKEN_SECRET_KEY, b"test-secret");

        for _ in 0..RATE_LIMIT_REQUESTS_DEFAULT {
            assert_eq!(
                handle_directline_request(
                    &token_request_with_ip("203.0.113.55")?,
                    &mut state,
                    &secrets,
                )
                .status,
                200
            );
        }

        let blocked = handle_directline_request(
            &token_request_with_ip("203.0.113.55")?,
            &mut state,
            &secrets,
        );
        assert_eq!(blocked.status, 429);
        let retry_after = header_value(&blocked, "Retry-After")
            .ok_or("Retry-After header must be present on 429")?;
        let secs: i64 = retry_after
            .parse()
            .map_err(|err| format!("Retry-After must be integer seconds: {err}"))?;
        assert!(
            (1..=RATE_LIMIT_WINDOW_SECONDS_DEFAULT).contains(&secs),
            "Retry-After should be within current window, got {secs}"
        );

        let body = decode_body(&blocked)?;
        assert_eq!(body["error"], "rate_limited");
        assert!(body.get("retry_after").is_some());
        Ok(())
    }

    #[test]
    fn rate_limit_config_override_raises_cap() {
        let mut state = InMemoryStateStore::new();
        let mut secrets = TestSecretStore::new();
        secrets.insert(TOKEN_SECRET_KEY, b"test-secret");

        // Config raises the per-window cap to 200; first 200 requests pass.
        let make = || HttpInV1 {
            method: "POST".into(),
            path: "/v3/directline/tokens/generate".into(),
            query: Some("env=default&tenant=default".into()),
            headers: vec![Header {
                name: "X-Forwarded-For".into(),
                value: "203.0.113.99".into(),
            }],
            body_b64: String::new(),
            route_hint: None,
            binding_id: None,
            config: Some(json!({ "rate_limit_requests": 200 })),
        };

        for _ in 0..200 {
            assert_eq!(
                handle_directline_request(&make(), &mut state, &secrets).status,
                200
            );
        }
        let blocked = handle_directline_request(&make(), &mut state, &secrets);
        assert_eq!(blocked.status, 429);
    }

    #[test]
    fn determine_subject_prefers_user_id_over_ip() -> Result<(), String> {
        let request = build_request(
            "POST",
            "/v3/directline/tokens/generate",
            None,
            Some(&json!({ "user": { "id": "guest-abc" } })),
            vec![Header {
                name: "X-Forwarded-For".into(),
                value: "203.0.113.10".into(),
            }],
        )?;
        let body = decode_json_body(&request).map_err(|out| format!("status {}", out.status))?;
        let subject = determine_rate_limit_subject(&request, &body);
        assert_eq!(subject, RateLimitSubject::User("guest-abc".to_string()));
        Ok(())
    }

    #[test]
    fn determine_subject_falls_back_to_anonymous_without_ip() -> Result<(), String> {
        let request = build_request("POST", "/v3/directline/tokens/generate", None, None, vec![])?;
        let body = decode_json_body(&request).map_err(|out| format!("status {}", out.status))?;
        let subject = determine_rate_limit_subject(&request, &body);
        assert_eq!(subject, RateLimitSubject::Anonymous);
        Ok(())
    }

    #[test]
    fn extract_client_ip_priority_order() {
        let headers = vec![
            Header {
                name: "X-Forwarded-For".into(),
                value: "1.1.1.1".into(),
            },
            Header {
                name: "X-Real-IP".into(),
                value: "2.2.2.2".into(),
            },
            Header {
                name: "True-Client-IP".into(),
                value: "3.3.3.3".into(),
            },
        ];
        // True-Client-IP wins (highest priority — Cloudflare/Akamai trust
        // boundary).
        assert_eq!(extract_client_ip(&headers).as_deref(), Some("3.3.3.3"));
    }

    #[test]
    fn empty_user_id_falls_through_to_ip_bucket() -> Result<(), String> {
        // Pre-fix bug: a body with `"user":{"id":""}` would be accepted as
        // the literal user id "" → `webchat:rate:tokens:..::` shared
        // bucket. Now we trim and treat empty as missing.
        let request = build_request(
            "POST",
            "/v3/directline/tokens/generate",
            None,
            Some(&json!({ "user": { "id": "   " } })),
            vec![Header {
                name: "X-Real-IP".into(),
                value: "203.0.113.77".into(),
            }],
        )?;
        let body = decode_json_body(&request).map_err(|out| format!("status {}", out.status))?;
        match determine_rate_limit_subject(&request, &body) {
            RateLimitSubject::Ip(_) => {}
            other => panic!("expected Ip bucket, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn validate_flow_hint_accepts_valid_id() {
        let headers = vec![Header {
            name: "X-Greentic-Flow".into(),
            value: "welcome-flow".into(),
        }];
        assert_eq!(
            extract_flow_hint(&headers),
            Some("welcome-flow".to_string())
        );
    }

    #[test]
    fn validate_flow_hint_trims_whitespace() {
        let headers = vec![Header {
            name: "X-Greentic-Flow".into(),
            value: "  my-flow  ".into(),
        }];
        assert_eq!(extract_flow_hint(&headers), Some("my-flow".to_string()));
    }

    #[test]
    fn validate_flow_hint_rejects_empty() {
        let headers = vec![Header {
            name: "X-Greentic-Flow".into(),
            value: "   ".into(),
        }];
        assert_eq!(extract_flow_hint(&headers), None);
    }

    #[test]
    fn validate_flow_hint_rejects_control_chars() {
        let headers = vec![Header {
            name: "X-Greentic-Flow".into(),
            value: "flow\x00id".into(),
        }];
        assert_eq!(extract_flow_hint(&headers), None);
    }

    #[test]
    fn validate_flow_hint_rejects_oversized() {
        let long = "a".repeat(FLOW_HINT_MAX_LEN + 1);
        let headers = vec![Header {
            name: "X-Greentic-Flow".into(),
            value: long,
        }];
        assert_eq!(extract_flow_hint(&headers), None);
    }

    #[test]
    fn validate_flow_hint_absent_header() {
        let headers = vec![Header {
            name: "Authorization".into(),
            value: "Bearer xyz".into(),
        }];
        assert_eq!(extract_flow_hint(&headers), None);
    }

    #[test]
    fn flow_hint_header_present_on_conversation_create() -> Result<(), String> {
        let mut state = InMemoryStateStore::new();
        let mut secrets = TestSecretStore::new();
        secrets.insert(TOKEN_SECRET_KEY, b"test-secret");

        let token_response = handle_directline_request(
            &build_request(
                "POST",
                "/v3/directline/tokens/generate",
                Some("env=default&tenant=default"),
                Some(&json!({"user": {"id": "alice"}})),
                vec![],
            )?,
            &mut state,
            &secrets,
        );
        let user_token = decode_body(&token_response)?["token"]
            .as_str()
            .ok_or("token")?
            .to_string();

        let conv_response = handle_directline_request(
            &build_request(
                "POST",
                "/v3/directline/conversations",
                None,
                None,
                vec![
                    Header {
                        name: "Authorization".into(),
                        value: format!("Bearer {user_token}"),
                    },
                    Header {
                        name: "X-Greentic-Flow".into(),
                        value: "onboarding-flow".into(),
                    },
                ],
            )?,
            &mut state,
            &secrets,
        );
        assert_eq!(conv_response.status, 201);
        assert_eq!(
            header_value(&conv_response, "X-Greentic-Flow"),
            Some("onboarding-flow"),
        );
        Ok(())
    }

    #[test]
    fn flow_hint_absent_means_no_header_on_conversation() -> Result<(), String> {
        let mut state = InMemoryStateStore::new();
        let mut secrets = TestSecretStore::new();
        secrets.insert(TOKEN_SECRET_KEY, b"test-secret");

        let token_response = handle_directline_request(
            &build_request(
                "POST",
                "/v3/directline/tokens/generate",
                Some("env=default&tenant=default"),
                Some(&json!({"user": {"id": "bob"}})),
                vec![],
            )?,
            &mut state,
            &secrets,
        );
        let user_token = decode_body(&token_response)?["token"]
            .as_str()
            .ok_or("token")?
            .to_string();

        let conv_response = handle_directline_request(
            &build_request(
                "POST",
                "/v3/directline/conversations",
                None,
                None,
                vec![Header {
                    name: "Authorization".into(),
                    value: format!("Bearer {user_token}"),
                }],
            )?,
            &mut state,
            &secrets,
        );
        assert_eq!(conv_response.status, 201);
        assert_eq!(header_value(&conv_response, "X-Greentic-Flow"), None);
        Ok(())
    }

    #[test]
    fn flow_binding_survives_into_activity_response() -> Result<(), String> {
        let mut state = InMemoryStateStore::new();
        let mut secrets = TestSecretStore::new();
        secrets.insert(TOKEN_SECRET_KEY, b"test-secret");

        // Generate token
        let token_response = handle_directline_request(
            &build_request(
                "POST",
                "/v3/directline/tokens/generate",
                Some("env=default&tenant=default"),
                Some(&json!({"user": {"id": "carol"}})),
                vec![],
            )?,
            &mut state,
            &secrets,
        );
        let user_token = decode_body(&token_response)?["token"]
            .as_str()
            .ok_or("token")?
            .to_string();

        // Create conversation with flow hint
        let conv_response = handle_directline_request(
            &build_request(
                "POST",
                "/v3/directline/conversations",
                None,
                None,
                vec![
                    Header {
                        name: "Authorization".into(),
                        value: format!("Bearer {user_token}"),
                    },
                    Header {
                        name: "X-Greentic-Flow".into(),
                        value: "support-flow".into(),
                    },
                ],
            )?,
            &mut state,
            &secrets,
        );
        assert_eq!(conv_response.status, 201);
        let conv_body = decode_body(&conv_response)?;
        let conversation_id = conv_body["conversationId"]
            .as_str()
            .ok_or("conversationId")?;
        let conv_token = conv_body["token"].as_str().ok_or("conv token")?.to_string();

        // Post activity — flow binding must surface via response header
        let activity_response = handle_directline_request(
            &build_request(
                "POST",
                &format!("/v3/directline/conversations/{conversation_id}/activities"),
                None,
                Some(&json!({"type": "message", "text": "hi", "from": {"id": "carol"}})),
                vec![Header {
                    name: "Authorization".into(),
                    value: format!("Bearer {conv_token}"),
                }],
            )?,
            &mut state,
            &secrets,
        );
        assert_eq!(activity_response.status, 201);
        assert_eq!(
            header_value(&activity_response, "X-Greentic-Flow"),
            Some("support-flow"),
        );
        Ok(())
    }

    #[test]
    fn no_flow_binding_means_no_header_on_activity() -> Result<(), String> {
        let mut state = InMemoryStateStore::new();
        let mut secrets = TestSecretStore::new();
        secrets.insert(TOKEN_SECRET_KEY, b"test-secret");

        let token_response = handle_directline_request(
            &build_request(
                "POST",
                "/v3/directline/tokens/generate",
                Some("env=default&tenant=default"),
                Some(&json!({"user": {"id": "dave"}})),
                vec![],
            )?,
            &mut state,
            &secrets,
        );
        let user_token = decode_body(&token_response)?["token"]
            .as_str()
            .ok_or("token")?
            .to_string();

        // Create conversation WITHOUT flow hint
        let conv_response = handle_directline_request(
            &build_request(
                "POST",
                "/v3/directline/conversations",
                None,
                None,
                vec![Header {
                    name: "Authorization".into(),
                    value: format!("Bearer {user_token}"),
                }],
            )?,
            &mut state,
            &secrets,
        );
        assert_eq!(conv_response.status, 201);
        let conv_body = decode_body(&conv_response)?;
        let conversation_id = conv_body["conversationId"]
            .as_str()
            .ok_or("conversationId")?;
        let conv_token = conv_body["token"].as_str().ok_or("conv token")?.to_string();

        let activity_response = handle_directline_request(
            &build_request(
                "POST",
                &format!("/v3/directline/conversations/{conversation_id}/activities"),
                None,
                Some(&json!({"type": "message", "text": "hello", "from": {"id": "dave"}})),
                vec![Header {
                    name: "Authorization".into(),
                    value: format!("Bearer {conv_token}"),
                }],
            )?,
            &mut state,
            &secrets,
        );
        assert_eq!(activity_response.status, 201);
        assert_eq!(header_value(&activity_response, "X-Greentic-Flow"), None);
        Ok(())
    }

    struct StaticJwks(String);
    impl JwksFetcher for StaticJwks {
        fn fetch(&self, _url: &str) -> Result<String, String> {
            Ok(self.0.clone())
        }
    }

    fn token_request_with_config(config: Value) -> HttpInV1 {
        HttpInV1 {
            method: "POST".to_string(),
            path: "/v3/directline/tokens/generate".to_string(),
            query: Some("env=default&tenant=default".to_string()),
            headers: vec![],
            body_b64: String::new(),
            route_hint: None,
            binding_id: None,
            config: Some(config),
        }
    }

    #[test]
    fn bearer_token_mints_an_identity_bound_direct_line_token() {
        let (access_token, jwks) = crate::directline::oidc_test_support::signed_fixture(
            "https://acme.greentic-id.com",
            "webchat-gui",
            "acme:users:7",
            "openid greentic.webchat",
            4_000_000_000,
        );
        let mut state = InMemoryStateStore::new();
        let mut secrets = TestSecretStore::new();
        secrets.insert(TOKEN_SECRET_KEY, b"test-signing-key");
        let mut request = token_request_with_config(json!({
            "oidc_issuer": "https://acme.greentic-id.com",
            "oidc_audience": "webchat-gui",
            "oidc_required_scope": "greentic.webchat",
        }));
        request.headers.push(Header {
            name: "Authorization".into(),
            value: format!("Bearer {access_token}"),
        });

        let response =
            handle_directline_request_with_jwks(&request, &mut state, &secrets, &StaticJwks(jwks));
        assert_eq!(response.status, 200);

        let body = decode_body(&response).expect("json body");
        let token = body["token"].as_str().expect("token string");
        let claims = verify_token(b"test-signing-key", token).expect("direct line token verifies");
        assert_eq!(claims.sub, "acme:users:7");
        assert!(claims.verified);
    }

    #[test]
    fn an_invalid_bearer_is_rejected_with_401() {
        let (_, jwks) = crate::directline::oidc_test_support::signed_fixture(
            "https://acme.greentic-id.com",
            "webchat-gui",
            "acme:users:7",
            "greentic.webchat",
            4_000_000_000,
        );
        let mut state = InMemoryStateStore::new();
        let mut secrets = TestSecretStore::new();
        secrets.insert(TOKEN_SECRET_KEY, b"test-signing-key");
        let mut request = token_request_with_config(json!({
            "oidc_issuer": "https://acme.greentic-id.com",
            "oidc_audience": "webchat-gui",
            "oidc_required_scope": "greentic.webchat",
        }));
        request.headers.push(Header {
            name: "Authorization".into(),
            value: "Bearer not-a-jwt".into(),
        });

        let response =
            handle_directline_request_with_jwks(&request, &mut state, &secrets, &StaticJwks(jwks));
        assert_eq!(response.status, 401);
    }

    #[test]
    fn no_bearer_still_mints_an_anonymous_token() {
        let mut state = InMemoryStateStore::new();
        let mut secrets = TestSecretStore::new();
        secrets.insert(TOKEN_SECRET_KEY, b"test-signing-key");
        let request = token_request_with_config(json!({
            "oidc_issuer": "https://acme.greentic-id.com",
        }));
        let response =
            handle_directline_request_with_jwks(&request, &mut state, &secrets, &NoJwksFetcher);
        assert_eq!(response.status, 200);
        let body = decode_body(&response).expect("json body");
        let claims = verify_token(b"test-signing-key", body["token"].as_str().expect("token"))
            .expect("verifies");
        assert!(!claims.verified);
    }

    #[test]
    fn bearer_without_oidc_configured_is_rejected_with_401() {
        // A stray/leftover Authorization header must not silently downgrade
        // to an anonymous mint when the tenant has no OIDC issuer configured
        // to verify against — that would hide a failed verification.
        let mut state = InMemoryStateStore::new();
        let mut secrets = TestSecretStore::new();
        secrets.insert(TOKEN_SECRET_KEY, b"test-signing-key");
        let mut request = token_request_with_config(json!({}));
        request.headers.push(Header {
            name: "Authorization".into(),
            value: "Bearer whatever".into(),
        });

        let response =
            handle_directline_request_with_jwks(&request, &mut state, &secrets, &NoJwksFetcher);
        assert_eq!(response.status, 401);
    }

    struct CountingJwks {
        document: String,
        calls: std::cell::Cell<usize>,
    }
    impl JwksFetcher for CountingJwks {
        fn fetch(&self, _url: &str) -> Result<String, String> {
            self.calls.set(self.calls.get() + 1);
            Ok(self.document.clone())
        }
    }

    #[test]
    fn jwks_is_fetched_once_across_two_mints() {
        let (access_token, jwks_doc) = crate::directline::oidc_test_support::signed_fixture(
            "https://acme.greentic-id.com",
            "webchat-gui",
            "acme:users:7",
            "greentic.webchat",
            4_000_000_000,
        );
        let fetcher = CountingJwks {
            document: jwks_doc,
            calls: std::cell::Cell::new(0),
        };
        let mut state = InMemoryStateStore::new();
        let mut secrets = TestSecretStore::new();
        secrets.insert(TOKEN_SECRET_KEY, b"test-signing-key");
        let mut request = token_request_with_config(json!({
            "oidc_issuer": "https://acme.greentic-id.com",
            "oidc_audience": "webchat-gui",
            "oidc_required_scope": "greentic.webchat",
            "rate_limit_requests": 100,
        }));
        request.headers.push(Header {
            name: "Authorization".into(),
            value: format!("Bearer {access_token}"),
        });

        for _ in 0..2 {
            let response =
                handle_directline_request_with_jwks(&request, &mut state, &secrets, &fetcher);
            assert_eq!(response.status, 200);
        }
        assert_eq!(fetcher.calls.get(), 1);
    }

    // --- C1: anonymous callers must not be able to self-declare an
    // issuer-subject-shaped id, and the DirectLine → envelope boundary must
    // carry whether the identity was actually verified. ---

    #[test]
    fn anonymous_mint_rejects_issuer_shaped_client_user_id() {
        let mut state = InMemoryStateStore::new();
        let mut secrets = TestSecretStore::new();
        secrets.insert(TOKEN_SECRET_KEY, b"test-signing-key");
        let request = build_request(
            "POST",
            "/v3/directline/tokens/generate",
            Some("env=default&tenant=default"),
            Some(&json!({"user": {"id": "acme:users:7"}})),
            vec![],
        )
        .expect("request");

        let response =
            handle_directline_request_with_jwks(&request, &mut state, &secrets, &NoJwksFetcher);
        assert_eq!(response.status, 400);
    }

    #[test]
    fn verified_conversation_carries_true_verified_header() {
        let (access_token, jwks) = crate::directline::oidc_test_support::signed_fixture(
            "https://acme.greentic-id.com",
            "webchat-gui",
            "acme:users:7",
            "greentic.webchat",
            4_000_000_000,
        );
        let mut state = InMemoryStateStore::new();
        let mut secrets = TestSecretStore::new();
        secrets.insert(TOKEN_SECRET_KEY, b"test-signing-key");
        let mut token_request = token_request_with_config(json!({
            "oidc_issuer": "https://acme.greentic-id.com",
            "oidc_audience": "webchat-gui",
            "oidc_required_scope": "greentic.webchat",
        }));
        token_request.headers.push(Header {
            name: "Authorization".into(),
            value: format!("Bearer {access_token}"),
        });
        let token_response = handle_directline_request_with_jwks(
            &token_request,
            &mut state,
            &secrets,
            &StaticJwks(jwks),
        );
        assert_eq!(token_response.status, 200);
        let user_token = decode_body(&token_response).expect("json body")["token"]
            .as_str()
            .expect("token")
            .to_string();

        let conv_request = build_request(
            "POST",
            "/v3/directline/conversations",
            None,
            None,
            vec![Header {
                name: "Authorization".into(),
                value: format!("Bearer {user_token}"),
            }],
        )
        .expect("request");
        let conv_response = handle_directline_request_with_jwks(
            &conv_request,
            &mut state,
            &secrets,
            &NoJwksFetcher,
        );
        assert_eq!(conv_response.status, 201);
        assert_eq!(
            header_value(&conv_response, "X-Greentic-User-Verified"),
            Some("true")
        );
    }

    #[test]
    fn anonymous_conversation_carries_false_verified_header() {
        let mut state = InMemoryStateStore::new();
        let mut secrets = TestSecretStore::new();
        secrets.insert(TOKEN_SECRET_KEY, b"test-signing-key");
        let token_response = handle_directline_request(
            &build_request(
                "POST",
                "/v3/directline/tokens/generate",
                Some("env=default&tenant=default"),
                Some(&json!({"user": {"id": "guest-abc"}})),
                vec![],
            )
            .expect("request"),
            &mut state,
            &secrets,
        );
        assert_eq!(token_response.status, 200);
        let user_token = decode_body(&token_response).expect("json body")["token"]
            .as_str()
            .expect("token")
            .to_string();

        let conv_response = handle_directline_request(
            &build_request(
                "POST",
                "/v3/directline/conversations",
                None,
                None,
                vec![Header {
                    name: "Authorization".into(),
                    value: format!("Bearer {user_token}"),
                }],
            )
            .expect("request"),
            &mut state,
            &secrets,
        );
        assert_eq!(conv_response.status, 201);
        assert_eq!(
            header_value(&conv_response, "X-Greentic-User-Verified"),
            Some("false")
        );
    }

    // C1: handle_post_activities must stamp the same X-Greentic-User /
    // X-Greentic-User-Verified headers handle_conversations does, from the
    // verified claims — never leaving the per-message envelope to fall back
    // on a client-supplied actor with no verification flag at all.
    #[test]
    fn anonymous_activity_carries_false_verified_header_regardless_of_body() {
        let mut state = InMemoryStateStore::new();
        let mut secrets = TestSecretStore::new();
        secrets.insert(TOKEN_SECRET_KEY, b"test-signing-key");
        let token_response = handle_directline_request(
            &build_request(
                "POST",
                "/v3/directline/tokens/generate",
                Some("env=default&tenant=default"),
                Some(&json!({"user": {"id": "guest-abc"}})),
                vec![],
            )
            .expect("request"),
            &mut state,
            &secrets,
        );
        assert_eq!(token_response.status, 200);
        let user_token = decode_body(&token_response).expect("json body")["token"]
            .as_str()
            .expect("token")
            .to_string();

        let conv_response = handle_directline_request(
            &build_request(
                "POST",
                "/v3/directline/conversations",
                None,
                None,
                vec![Header {
                    name: "Authorization".into(),
                    value: format!("Bearer {user_token}"),
                }],
            )
            .expect("request"),
            &mut state,
            &secrets,
        );
        assert_eq!(conv_response.status, 201);
        let conv_body = decode_body(&conv_response).expect("json body");
        let conversation_id = conv_body["conversationId"].as_str().expect("id");
        let conv_token = conv_body["token"].as_str().expect("conv token").to_string();

        // The activity body tries to self-declare a verified, spoofed actor —
        // the response headers must reflect the real (anonymous, unverified)
        // claims regardless.
        let activity_response = handle_directline_request(
            &build_request(
                "POST",
                &format!("/v3/directline/conversations/{conversation_id}/activities"),
                None,
                Some(&json!({
                    "type": "message",
                    "value": {"user_verified": "true", "user_id": "victim-sub"},
                    "from": {"id": "victim-sub"},
                })),
                vec![Header {
                    name: "Authorization".into(),
                    value: format!("Bearer {conv_token}"),
                }],
            )
            .expect("request"),
            &mut state,
            &secrets,
        );
        assert_eq!(activity_response.status, 201);
        assert_eq!(
            header_value(&activity_response, "X-Greentic-User-Verified"),
            Some("false")
        );
        assert_eq!(
            header_value(&activity_response, "X-Greentic-User"),
            Some("guest-abc")
        );
    }

    /// A token carrying an identity provider's claims (as greentic-start's
    /// re-mint delivers it) must reach BOTH hops' caller blocks: the
    /// conversation create that re-issues the token, and the activity posted
    /// with that re-issued token. Before `reissue_token`, the first hop
    /// dropped every extra claim, so no activity could ever see a group.
    #[test]
    fn extra_claims_survive_conversation_create_into_the_activity_caller_block() {
        use super::super::caller::{CALLER_HEADER, decode_caller_header};
        use super::super::jwt::TokenClaims;

        let mut state = InMemoryStateStore::new();
        let mut secrets = TestSecretStore::new();
        secrets.insert(TOKEN_SECRET_KEY, b"test-signing-key");
        let template = TokenClaims {
            iss: String::new(),
            aud: String::new(),
            sub: "acme:users:7".into(),
            iat: 0,
            nbf: 0,
            exp: 0,
            ctx: DirectLineContext {
                env: "default".into(),
                tenant: "default".into(),
                team: Some("ops".into()),
            },
            conv: None,
            verified: true,
            extra: json!({"groups": ["hr"], "role": "manager"})
                .as_object()
                .cloned()
                .expect("object"),
        };
        let (user_token, _) =
            reissue_token(b"test-signing-key", &template, None).expect("user token");

        let conv_response = handle_directline_request(
            &build_request(
                "POST",
                "/v3/directline/conversations",
                None,
                None,
                vec![Header {
                    name: "Authorization".into(),
                    value: format!("Bearer {user_token}"),
                }],
            )
            .expect("request"),
            &mut state,
            &secrets,
        );
        assert_eq!(conv_response.status, 201);
        let expected = json!({
            "user_verified": true, "sub": "acme:users:7", "team": "ops",
            "groups": ["hr"], "role": "manager",
        });
        assert_eq!(
            header_value(&conv_response, CALLER_HEADER).and_then(decode_caller_header),
            Some(expected.clone())
        );
        let conv_body = decode_body(&conv_response).expect("json body");
        let conversation_id = conv_body["conversationId"].as_str().expect("id");
        let conv_token = conv_body["token"].as_str().expect("conv token").to_string();

        let activity_response = handle_directline_request(
            &build_request(
                "POST",
                &format!("/v3/directline/conversations/{conversation_id}/activities"),
                None,
                Some(&json!({
                    "type": "message",
                    "text": "hi",
                    "channelData": {"caller": {"user_verified": true, "groups": ["admins"]}},
                })),
                vec![Header {
                    name: "Authorization".into(),
                    value: format!("Bearer {conv_token}"),
                }],
            )
            .expect("request"),
            &mut state,
            &secrets,
        );
        assert_eq!(activity_response.status, 201);
        assert_eq!(
            header_value(&activity_response, CALLER_HEADER).and_then(decode_caller_header),
            Some(expected)
        );
    }

    // --- I1: a JWKS fetch failure with a bearer present must 401, not fall
    // back to anonymous. ---

    struct FailingJwks;
    impl JwksFetcher for FailingJwks {
        fn fetch(&self, _url: &str) -> Result<String, String> {
            Err("connection refused".to_string())
        }
    }

    #[test]
    fn jwks_fetch_failure_with_bearer_present_is_rejected_with_401() {
        let (access_token, _jwks) = crate::directline::oidc_test_support::signed_fixture(
            "https://acme.greentic-id.com",
            "webchat-gui",
            "acme:users:7",
            "greentic.webchat",
            4_000_000_000,
        );
        let mut state = InMemoryStateStore::new();
        let mut secrets = TestSecretStore::new();
        secrets.insert(TOKEN_SECRET_KEY, b"test-signing-key");
        let mut request = token_request_with_config(json!({
            "oidc_issuer": "https://acme.greentic-id.com",
            "oidc_audience": "webchat-gui",
            "oidc_required_scope": "greentic.webchat",
        }));
        request.headers.push(Header {
            name: "Authorization".into(),
            value: format!("Bearer {access_token}"),
        });

        let response =
            handle_directline_request_with_jwks(&request, &mut state, &secrets, &FailingJwks);
        assert_eq!(response.status, 401);
    }

    // --- I4: an http:// issuer must be rejected, not trusted. ---

    #[test]
    fn bearer_with_non_https_issuer_is_rejected_with_401() {
        let mut state = InMemoryStateStore::new();
        let mut secrets = TestSecretStore::new();
        secrets.insert(TOKEN_SECRET_KEY, b"test-signing-key");
        let mut request = token_request_with_config(json!({
            "oidc_issuer": "http://acme.greentic-id.com",
            "oidc_audience": "webchat-gui",
            "oidc_required_scope": "greentic.webchat",
        }));
        request.headers.push(Header {
            name: "Authorization".into(),
            value: "Bearer whatever".into(),
        });

        let response =
            handle_directline_request_with_jwks(&request, &mut state, &secrets, &NoJwksFetcher);
        assert_eq!(response.status, 401);
    }

    // --- I5: a rotated signing key must not hard-401 every caller until the
    // cache TTL expires — one bounded retry, bypassing the cache, must
    // recover. ---

    struct RotatingJwks {
        old: String,
        new: String,
        calls: std::cell::Cell<usize>,
    }
    impl JwksFetcher for RotatingJwks {
        fn fetch(&self, _url: &str) -> Result<String, String> {
            let n = self.calls.get();
            self.calls.set(n + 1);
            Ok(if n == 0 {
                self.old.clone()
            } else {
                self.new.clone()
            })
        }
    }

    #[test]
    fn jwks_cache_invalidates_and_retries_once_after_key_rotation() {
        let (token1, jwks1) = crate::directline::oidc_test_support::signed_fixture(
            "https://acme.greentic-id.com",
            "webchat-gui",
            "acme:users:1",
            "greentic.webchat",
            4_000_000_000,
        );
        let (token2, jwks2) = crate::directline::oidc_test_support::signed_fixture(
            "https://acme.greentic-id.com",
            "webchat-gui",
            "acme:users:2",
            "greentic.webchat",
            4_000_000_000,
        );
        let fetcher = RotatingJwks {
            old: jwks1,
            new: jwks2,
            calls: std::cell::Cell::new(0),
        };
        let mut state = InMemoryStateStore::new();
        let mut secrets = TestSecretStore::new();
        secrets.insert(TOKEN_SECRET_KEY, b"test-signing-key");
        let cfg = json!({
            "oidc_issuer": "https://acme.greentic-id.com",
            "oidc_audience": "webchat-gui",
            "oidc_required_scope": "greentic.webchat",
            "rate_limit_requests": 100,
        });

        // First mint populates the cache with the pre-rotation JWKS (key 1).
        let mut req1 = token_request_with_config(cfg.clone());
        req1.headers.push(Header {
            name: "Authorization".into(),
            value: format!("Bearer {token1}"),
        });
        let resp1 = handle_directline_request_with_jwks(&req1, &mut state, &secrets, &fetcher);
        assert_eq!(resp1.status, 200);

        // Second mint presents a token signed by the new (post-rotation)
        // key. The cached JWKS won't contain it — UnknownKey should trigger
        // exactly one bounded refetch, which succeeds.
        let mut req2 = token_request_with_config(cfg);
        req2.headers.push(Header {
            name: "Authorization".into(),
            value: format!("Bearer {token2}"),
        });
        let resp2 = handle_directline_request_with_jwks(&req2, &mut state, &secrets, &fetcher);
        assert_eq!(resp2.status, 200);
        let body = decode_body(&resp2).expect("json body");
        let claims = verify_token(b"test-signing-key", body["token"].as_str().expect("token"))
            .expect("verifies");
        assert_eq!(claims.sub, "acme:users:2");
        assert_eq!(fetcher.calls.get(), 2);
    }

    // --- I2: `verified` must survive re-minting through handle_conversations,
    // handle_refresh_token, and handle_reconnect_conversation, in both
    // directions — a verified session must not be downgraded, and an
    // anonymous session must not be laundered into a verified one. ---

    fn mint_verified_user_token(
        state: &mut InMemoryStateStore,
        secrets: &TestSecretStore,
    ) -> String {
        let (access_token, jwks) = crate::directline::oidc_test_support::signed_fixture(
            "https://acme.greentic-id.com",
            "webchat-gui",
            "acme:users:7",
            "greentic.webchat",
            4_000_000_000,
        );
        let mut request = token_request_with_config(json!({
            "oidc_issuer": "https://acme.greentic-id.com",
            "oidc_audience": "webchat-gui",
            "oidc_required_scope": "greentic.webchat",
        }));
        request.headers.push(Header {
            name: "Authorization".into(),
            value: format!("Bearer {access_token}"),
        });
        let response =
            handle_directline_request_with_jwks(&request, state, secrets, &StaticJwks(jwks));
        assert_eq!(response.status, 200);
        decode_body(&response).expect("json body")["token"]
            .as_str()
            .expect("token")
            .to_string()
    }

    fn mint_anonymous_user_token(
        state: &mut InMemoryStateStore,
        secrets: &TestSecretStore,
    ) -> String {
        let response = handle_directline_request(
            &build_request(
                "POST",
                "/v3/directline/tokens/generate",
                Some("env=default&tenant=default"),
                Some(&json!({"user": {"id": "guest-abc"}})),
                vec![],
            )
            .expect("request"),
            state,
            secrets,
        );
        assert_eq!(response.status, 200);
        decode_body(&response).expect("json body")["token"]
            .as_str()
            .expect("token")
            .to_string()
    }

    #[test]
    fn verified_flag_survives_conversation_creation() {
        let mut state = InMemoryStateStore::new();
        let mut secrets = TestSecretStore::new();
        secrets.insert(TOKEN_SECRET_KEY, b"test-signing-key");
        let user_token = mint_verified_user_token(&mut state, &secrets);

        let conv_response = handle_directline_request(
            &build_request(
                "POST",
                "/v3/directline/conversations",
                None,
                None,
                vec![Header {
                    name: "Authorization".into(),
                    value: format!("Bearer {user_token}"),
                }],
            )
            .expect("request"),
            &mut state,
            &secrets,
        );
        assert_eq!(conv_response.status, 201);
        let conv_token = decode_body(&conv_response).expect("json body")["token"]
            .as_str()
            .expect("token")
            .to_string();
        let claims = verify_token(b"test-signing-key", &conv_token).expect("verifies");
        assert!(claims.verified);
    }

    #[test]
    fn unverified_flag_survives_conversation_creation() {
        let mut state = InMemoryStateStore::new();
        let mut secrets = TestSecretStore::new();
        secrets.insert(TOKEN_SECRET_KEY, b"test-signing-key");
        let user_token = mint_anonymous_user_token(&mut state, &secrets);

        let conv_response = handle_directline_request(
            &build_request(
                "POST",
                "/v3/directline/conversations",
                None,
                None,
                vec![Header {
                    name: "Authorization".into(),
                    value: format!("Bearer {user_token}"),
                }],
            )
            .expect("request"),
            &mut state,
            &secrets,
        );
        assert_eq!(conv_response.status, 201);
        let conv_token = decode_body(&conv_response).expect("json body")["token"]
            .as_str()
            .expect("token")
            .to_string();
        let claims = verify_token(b"test-signing-key", &conv_token).expect("verifies");
        assert!(!claims.verified);
    }

    #[test]
    fn verified_flag_survives_token_refresh() {
        let mut state = InMemoryStateStore::new();
        let mut secrets = TestSecretStore::new();
        secrets.insert(TOKEN_SECRET_KEY, b"test-signing-key");
        let user_token = mint_verified_user_token(&mut state, &secrets);

        let refresh_response = handle_directline_request(
            &build_request(
                "POST",
                "/v3/directline/tokens/refresh",
                None,
                None,
                vec![Header {
                    name: "Authorization".into(),
                    value: format!("Bearer {user_token}"),
                }],
            )
            .expect("request"),
            &mut state,
            &secrets,
        );
        assert_eq!(refresh_response.status, 200);
        let refreshed_token = decode_body(&refresh_response).expect("json body")["token"]
            .as_str()
            .expect("token")
            .to_string();
        let claims = verify_token(b"test-signing-key", &refreshed_token).expect("verifies");
        assert!(claims.verified);
    }

    #[test]
    fn unverified_flag_survives_token_refresh() {
        let mut state = InMemoryStateStore::new();
        let mut secrets = TestSecretStore::new();
        secrets.insert(TOKEN_SECRET_KEY, b"test-signing-key");
        let user_token = mint_anonymous_user_token(&mut state, &secrets);

        let refresh_response = handle_directline_request(
            &build_request(
                "POST",
                "/v3/directline/tokens/refresh",
                None,
                None,
                vec![Header {
                    name: "Authorization".into(),
                    value: format!("Bearer {user_token}"),
                }],
            )
            .expect("request"),
            &mut state,
            &secrets,
        );
        assert_eq!(refresh_response.status, 200);
        let refreshed_token = decode_body(&refresh_response).expect("json body")["token"]
            .as_str()
            .expect("token")
            .to_string();
        let claims = verify_token(b"test-signing-key", &refreshed_token).expect("verifies");
        assert!(!claims.verified);
    }

    #[test]
    fn verified_flag_survives_reconnect() {
        let mut state = InMemoryStateStore::new();
        let mut secrets = TestSecretStore::new();
        secrets.insert(TOKEN_SECRET_KEY, b"test-signing-key");
        let user_token = mint_verified_user_token(&mut state, &secrets);

        let conv_response = handle_directline_request(
            &build_request(
                "POST",
                "/v3/directline/conversations",
                None,
                None,
                vec![Header {
                    name: "Authorization".into(),
                    value: format!("Bearer {user_token}"),
                }],
            )
            .expect("request"),
            &mut state,
            &secrets,
        );
        assert_eq!(conv_response.status, 201);
        let conv_body = decode_body(&conv_response).expect("json body");
        let conversation_id = conv_body["conversationId"]
            .as_str()
            .expect("conversationId");
        let conv_token = conv_body["token"].as_str().expect("token").to_string();

        let reconnect_response = handle_directline_request(
            &build_request(
                "GET",
                &format!("/v3/directline/conversations/{conversation_id}"),
                None,
                None,
                vec![Header {
                    name: "Authorization".into(),
                    value: format!("Bearer {conv_token}"),
                }],
            )
            .expect("request"),
            &mut state,
            &secrets,
        );
        assert_eq!(reconnect_response.status, 200);
        let reconnected_token = decode_body(&reconnect_response).expect("json body")["token"]
            .as_str()
            .expect("token")
            .to_string();
        let claims = verify_token(b"test-signing-key", &reconnected_token).expect("verifies");
        assert!(claims.verified);
    }

    #[test]
    fn unverified_flag_survives_reconnect() {
        let mut state = InMemoryStateStore::new();
        let mut secrets = TestSecretStore::new();
        secrets.insert(TOKEN_SECRET_KEY, b"test-signing-key");
        let user_token = mint_anonymous_user_token(&mut state, &secrets);

        let conv_response = handle_directline_request(
            &build_request(
                "POST",
                "/v3/directline/conversations",
                None,
                None,
                vec![Header {
                    name: "Authorization".into(),
                    value: format!("Bearer {user_token}"),
                }],
            )
            .expect("request"),
            &mut state,
            &secrets,
        );
        assert_eq!(conv_response.status, 201);
        let conv_body = decode_body(&conv_response).expect("json body");
        let conversation_id = conv_body["conversationId"]
            .as_str()
            .expect("conversationId");
        let conv_token = conv_body["token"].as_str().expect("token").to_string();

        let reconnect_response = handle_directline_request(
            &build_request(
                "GET",
                &format!("/v3/directline/conversations/{conversation_id}"),
                None,
                None,
                vec![Header {
                    name: "Authorization".into(),
                    value: format!("Bearer {conv_token}"),
                }],
            )
            .expect("request"),
            &mut state,
            &secrets,
        );
        assert_eq!(reconnect_response.status, 200);
        let reconnected_token = decode_body(&reconnect_response).expect("json body")["token"]
            .as_str()
            .expect("token")
            .to_string();
        let claims = verify_token(b"test-signing-key", &reconnected_token).expect("verifies");
        assert!(!claims.verified);
    }

    // --- Follow-up on I5: the bounded UnknownKey retry must itself be rate
    // limited per issuer. Otherwise a flood of cheap forged tokens (bad kid,
    // no valid signature/claims required to reach UnknownKey) forces one
    // outbound JWKS fetch per request — the issuer, not us, is the victim. ---

    #[test]
    fn unknown_kid_requests_within_cooldown_trigger_only_one_refetch() {
        // A token whose kid the cached JWKS does not carry — every
        // verification attempt lands on `OidcError::UnknownKey`.
        let (token, _own_jwks) = crate::directline::oidc_test_support::signed_fixture(
            "https://acme.greentic-id.com",
            "webchat-gui",
            "acme:users:7",
            "greentic.webchat",
            4_000_000_000,
        );
        let (_, wrong_jwks) = crate::directline::oidc_test_support::signed_fixture(
            "https://acme.greentic-id.com",
            "webchat-gui",
            "someone-else",
            "greentic.webchat",
            4_000_000_000,
        );

        let mut state = InMemoryStateStore::new();
        let mut secrets = TestSecretStore::new();
        secrets.insert(TOKEN_SECRET_KEY, b"test-signing-key");
        let jwks_url = "https://acme.greentic-id.com/jwks.json";
        let seed_now = Utc::now().timestamp();
        // Seed the cache directly so the first request's `load_jwks` is a
        // cache hit, not a fetcher call — isolates the count to only the
        // gated `refetch_jwks` calls under test.
        store_jwks_cache(&mut state, jwks_url, &wrong_jwks, seed_now);

        let fetcher = CountingJwks {
            document: wrong_jwks,
            calls: std::cell::Cell::new(0),
        };
        let mut request = token_request_with_config(json!({
            "oidc_issuer": "https://acme.greentic-id.com",
            "oidc_audience": "webchat-gui",
            "oidc_required_scope": "greentic.webchat",
            "rate_limit_requests": 100,
        }));
        request.headers.push(Header {
            name: "Authorization".into(),
            value: format!("Bearer {token}"),
        });

        for _ in 0..5 {
            let response =
                handle_directline_request_with_jwks(&request, &mut state, &secrets, &fetcher);
            assert_eq!(response.status, 401);
        }
        assert_eq!(
            fetcher.calls.get(),
            1,
            "only the first UnknownKey should trigger a refetch inside the cooldown window"
        );
    }

    #[test]
    fn unknown_kid_request_after_cooldown_elapsed_does_refetch() {
        let (token, _own_jwks) = crate::directline::oidc_test_support::signed_fixture(
            "https://acme.greentic-id.com",
            "webchat-gui",
            "acme:users:7",
            "greentic.webchat",
            4_000_000_000,
        );
        let (_, wrong_jwks) = crate::directline::oidc_test_support::signed_fixture(
            "https://acme.greentic-id.com",
            "webchat-gui",
            "someone-else",
            "greentic.webchat",
            4_000_000_000,
        );

        let mut state = InMemoryStateStore::new();
        let mut secrets = TestSecretStore::new();
        secrets.insert(TOKEN_SECRET_KEY, b"test-signing-key");
        let jwks_url = "https://acme.greentic-id.com/jwks.json";
        let now = Utc::now().timestamp();
        store_jwks_cache(&mut state, jwks_url, &wrong_jwks, now);

        let fetcher = CountingJwks {
            document: wrong_jwks,
            calls: std::cell::Cell::new(0),
        };
        let mut request = token_request_with_config(json!({
            "oidc_issuer": "https://acme.greentic-id.com",
            "oidc_audience": "webchat-gui",
            "oidc_required_scope": "greentic.webchat",
            "rate_limit_requests": 100,
        }));
        request.headers.push(Header {
            name: "Authorization".into(),
            value: format!("Bearer {token}"),
        });

        let first = handle_directline_request_with_jwks(&request, &mut state, &secrets, &fetcher);
        assert_eq!(first.status, 401);
        assert_eq!(
            fetcher.calls.get(),
            1,
            "first UnknownKey should refetch once"
        );

        // `handle_tokens` derives `now` from the real clock, which this test
        // cannot advance directly. Instead we rewrite the persisted
        // last-attempt record to look 61 minutes old — equivalent in effect
        // to advancing the clock past the 60s cooldown from that record's
        // point of view.
        let stale_attempt = JwksRefetchAttempt {
            attempted_at: now - 61 * 60,
        };
        let bytes = serde_json::to_vec(&stale_attempt).expect("serialize attempt");
        state
            .write(&jwks_refetch_key(jwks_url), &bytes)
            .expect("seed stale cooldown record");

        let second = handle_directline_request_with_jwks(&request, &mut state, &secrets, &fetcher);
        assert_eq!(second.status, 401);
        assert_eq!(
            fetcher.calls.get(),
            2,
            "a request after the cooldown elapsed must refetch again"
        );
    }

    // --- The cooldown gate must fail closed on state-store errors, not just
    // cap the healthy-store case. `Err(_)` from a read is not evidence that
    // no attempt was made, and a write failure means the attempt can't be
    // recorded — either way the fetch must not happen, or a store an
    // attacker can make unhealthy becomes a way to switch the cap off. ---

    /// Wraps `InMemoryStateStore` but can be told to fail `read` and/or
    /// `write` for keys matching a given prefix, leaving every other key
    /// unaffected — lets a test fail only the refetch-cooldown record while
    /// the JWKS document cache continues to behave normally.
    struct SelectiveFailStore {
        inner: InMemoryStateStore,
        fail_read_prefix: Option<&'static str>,
        fail_write_prefix: Option<&'static str>,
    }

    impl SelectiveFailStore {
        fn new() -> Self {
            Self {
                inner: InMemoryStateStore::new(),
                fail_read_prefix: None,
                fail_write_prefix: None,
            }
        }
    }

    impl StateStore for SelectiveFailStore {
        fn read(&mut self, key: &str) -> Result<Option<Vec<u8>>, String> {
            if self
                .fail_read_prefix
                .is_some_and(|prefix| key.starts_with(prefix))
            {
                return Err("simulated read failure".to_string());
            }
            self.inner.read(key)
        }

        fn write(&mut self, key: &str, value: &[u8]) -> Result<(), String> {
            if self
                .fail_write_prefix
                .is_some_and(|prefix| key.starts_with(prefix))
            {
                return Err("simulated write failure".to_string());
            }
            self.inner.write(key, value)
        }
    }

    #[test]
    fn cooldown_read_error_denies_refetch_and_produces_zero_fetches() {
        let (token, _own_jwks) = crate::directline::oidc_test_support::signed_fixture(
            "https://acme.greentic-id.com",
            "webchat-gui",
            "acme:users:7",
            "greentic.webchat",
            4_000_000_000,
        );
        let (_, wrong_jwks) = crate::directline::oidc_test_support::signed_fixture(
            "https://acme.greentic-id.com",
            "webchat-gui",
            "someone-else",
            "greentic.webchat",
            4_000_000_000,
        );

        let mut store = SelectiveFailStore::new();
        let jwks_url = "https://acme.greentic-id.com/jwks.json";
        // Seed the JWKS document cache before turning on the failure, so the
        // document read (a different key prefix) keeps succeeding — this
        // isolates the assertion to the refetch-cooldown read path only.
        store_jwks_cache(&mut store, jwks_url, &wrong_jwks, Utc::now().timestamp());
        store.fail_read_prefix = Some("webchat:jwks:refetch:");

        let mut secrets = TestSecretStore::new();
        secrets.insert(TOKEN_SECRET_KEY, b"test-signing-key");
        let fetcher = CountingJwks {
            document: wrong_jwks,
            calls: std::cell::Cell::new(0),
        };
        let mut request = token_request_with_config(json!({
            "oidc_issuer": "https://acme.greentic-id.com",
            "oidc_audience": "webchat-gui",
            "oidc_required_scope": "greentic.webchat",
            "rate_limit_requests": 100,
        }));
        request.headers.push(Header {
            name: "Authorization".into(),
            value: format!("Bearer {token}"),
        });

        for _ in 0..5 {
            let response =
                handle_directline_request_with_jwks(&request, &mut store, &secrets, &fetcher);
            assert_eq!(response.status, 401);
        }
        assert_eq!(
            fetcher.calls.get(),
            0,
            "a cooldown-record read error must deny the refetch, not fail open"
        );
    }

    #[test]
    fn cooldown_write_error_denies_refetch_and_produces_zero_fetches() {
        let (token, _own_jwks) = crate::directline::oidc_test_support::signed_fixture(
            "https://acme.greentic-id.com",
            "webchat-gui",
            "acme:users:7",
            "greentic.webchat",
            4_000_000_000,
        );
        let (_, wrong_jwks) = crate::directline::oidc_test_support::signed_fixture(
            "https://acme.greentic-id.com",
            "webchat-gui",
            "someone-else",
            "greentic.webchat",
            4_000_000_000,
        );

        let mut store = SelectiveFailStore::new();
        let jwks_url = "https://acme.greentic-id.com/jwks.json";
        store_jwks_cache(&mut store, jwks_url, &wrong_jwks, Utc::now().timestamp());
        // Reads still succeed (including the "no prior attempt" read on the
        // cooldown key) — only persisting the attempt fails.
        store.fail_write_prefix = Some("webchat:jwks:refetch:");

        let mut secrets = TestSecretStore::new();
        secrets.insert(TOKEN_SECRET_KEY, b"test-signing-key");
        let fetcher = CountingJwks {
            document: wrong_jwks,
            calls: std::cell::Cell::new(0),
        };
        let mut request = token_request_with_config(json!({
            "oidc_issuer": "https://acme.greentic-id.com",
            "oidc_audience": "webchat-gui",
            "oidc_required_scope": "greentic.webchat",
            "rate_limit_requests": 100,
        }));
        request.headers.push(Header {
            name: "Authorization".into(),
            value: format!("Bearer {token}"),
        });

        for _ in 0..5 {
            let response =
                handle_directline_request_with_jwks(&request, &mut store, &secrets, &fetcher);
            assert_eq!(response.status, 401);
        }
        assert_eq!(
            fetcher.calls.get(),
            0,
            "an unrecordable attempt must deny the refetch, not fail open"
        );
    }

    #[test]
    fn validate_channel_data_accepts_ordinary_rag_citations() {
        let body = json!({
            "type": "message",
            "channelData": {
                "rag": {
                    "type": "rag",
                    "model": "claude-sonnet-5",
                    "citations": [{"index": 1, "doc": "Metro Design v3.2", "source_file": "metro-design-v3.2.pdf"}]
                }
            }
        });
        assert!(validate_channel_data(&body).is_ok());
    }

    #[test]
    fn validate_channel_data_rejects_oversized_payload() {
        let body = json!({
            "type": "message",
            "channelData": {
                "rag": { "type": "rag", "citations": [{"index": 1, "doc": "Metro Design v3.2", "text": "x".repeat(MAX_CHANNEL_DATA_BYTES + 1)}] }
            }
        });
        let response = validate_channel_data(&body).expect_err("oversized channelData");
        assert_eq!(response.status, 400);
    }
}
