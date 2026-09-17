//! Inbound HTTP / ingress path for the webchat provider.
//!
//! Two entry points:
//! - `handle_ingest` — legacy raw JSON ingest (used by the `ingest` op).
//! - `ingest_http`   — the main HTTP ingress router: dispatches OAuth,
//!   Direct Line (`/v3/directline/*`, `/token` shorthand), and a generic
//!   webhook fallback. This is where conversation/activity envelopes are
//!   emitted back to the operator so flows can auto-start.

use base64::{Engine as _, engine::general_purpose};
use greentic_types::messaging::universal_dto::{Header, HttpInV1, HttpOutV1};
use provider_common::helpers::json_bytes;
use provider_common::http_compat::{http_out_error, http_out_v1_bytes, parse_operator_http_in};
use provider_common::lifecycle_events::{mark_user_entered, user_entered_idempotency_key};
use provider_common::redact;
use provider_common::telemetry::{self, Field, Level, field};
use serde_json::{Value, json};
use std::collections::BTreeMap;

use crate::PROVIDER_TYPE;

use crate::directline::caller::{CALLER_EXT_KEY, CALLER_HEADER, decode_caller_header};
use crate::directline::{
    ConfigAwareSecretStore, HostJwksFetcher, HostStateStore, handle_directline_request_with_jwks,
};

use super::envelope::{build_webchat_envelope, build_webchat_envelope_with_ctx};
use super::helpers::{
    decode_body_json, extract_activity_text, extract_text, non_empty_string, route_from_value,
    tenant_channel_from_value, user_from_value,
};
use super::oauth::{handle_auth_config, handle_oauth_token_exchange};

pub(crate) fn handle_ingest(input_json: &[u8]) -> Vec<u8> {
    let parsed: Value = match serde_json::from_slice(input_json) {
        Ok(val) => val,
        Err(err) => {
            return json_bytes(&json!({"ok": false, "error": format!("invalid json: {err}")}));
        }
    };
    let text = parsed
        .get("text")
        .or_else(|| parsed.get("message"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let user = parsed
        .get("user_id")
        .or_else(|| parsed.get("from"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let envelope = json!({
        "from": user,
        "text": text,
        "raw": parsed,
    });
    json_bytes(&json!({"ok": true, "envelope": envelope}))
}

pub(crate) fn ingest_http(input_json: &[u8]) -> Vec<u8> {
    // Try native greentic-types format first, fall back to operator format
    let request = match serde_json::from_slice::<HttpInV1>(input_json) {
        Ok(req) => req,
        Err(_) => match parse_operator_http_in(input_json) {
            Ok(req) => req,
            Err(err) => return http_out_error(400, &format!("invalid http input: {err}")),
        },
    };
    // Serve OAuth configuration for the frontend SPA.
    if request.path.ends_with("/auth/config") && request.method.eq_ignore_ascii_case("GET") {
        return handle_auth_config(&request);
    }
    // Server-side OAuth token exchange — keeps client_secret off the browser.
    if request.path.ends_with("/oauth/token-exchange")
        && request.method.eq_ignore_ascii_case("POST")
    {
        return handle_oauth_token_exchange(&request);
    }

    // Translate shorthand /token path to the canonical DirectLine token endpoint.
    // The operator routes /v1/messaging/webchat/{tenant}/token through generic ingress,
    // so the provider component must recognize and forward it.
    if request.path.ends_with("/token") && !request.path.contains("/v3/directline") {
        let token_request = HttpInV1 {
            method: "POST".to_string(),
            path: "/v3/directline/tokens/generate".to_string(),
            query: request.query.clone(),
            headers: request.headers.clone(),
            body_b64: request.body_b64.clone(),
            route_hint: request.route_hint.clone(),
            binding_id: request.binding_id.clone(),
            config: request.config.clone(),
        };
        let mut state_driver = HostStateStore;
        let secrets_driver = ConfigAwareSecretStore::new(request.config.clone());
        let out = handle_directline_request_with_jwks(
            &token_request,
            &mut state_driver,
            &secrets_driver,
            &HostJwksFetcher,
        );
        return http_out_v1_bytes(&out);
    }

    // Extract Direct Line sub-path from operator-prefixed or direct paths.
    // Operator forwards full URI like /messaging/ingress/webchat/default/_/v3/directline/...
    if let Some(offset) = request.path.find("/v3/directline") {
        return handle_directline_path(&request, offset);
    }

    // Generic webhook fallback: normalise the raw body + emit an envelope.
    let body_bytes = match general_purpose::STANDARD.decode(&request.body_b64) {
        Ok(bytes) => bytes,
        Err(err) => return http_out_error(400, &format!("invalid body encoding: {err}")),
    };
    let body_val: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    let text = extract_text(&body_val);
    let user = user_from_value(&body_val);
    let route =
        non_empty_string(request.route_hint.as_deref()).or_else(|| route_from_value(&body_val));
    let tenant_channel_id = tenant_channel_from_value(&body_val);
    let envelope = build_webchat_envelope(
        text.clone(),
        user.clone(),
        route.clone(),
        tenant_channel_id.clone(),
    );
    let normalized = json!({
        "ok": true,
        "event": body_val,
        "route": route,
        "tenant_channel_id": tenant_channel_id,
    });
    let normalized_bytes = serde_json::to_vec(&normalized).unwrap_or_else(|_| b"{}".to_vec());
    let out = HttpOutV1 {
        status: 200,
        headers: Vec::new(),
        body_b64: general_purpose::STANDARD.encode(&normalized_bytes),
        events: vec![envelope],
    };
    http_out_v1_bytes(&out)
}

/// Handle a Direct Line sub-request (anything under `/v3/directline/...`).
///
/// Delegates to the directline crate for the actual protocol response, then
/// post-processes to emit `ChannelMessageEnvelope` events for `POST conversations`
/// (auto-start) and `POST activities` (forward user messages into the flow engine).
fn handle_directline_path(request: &HttpInV1, offset: usize) -> Vec<u8> {
    let dl_path = &request.path[offset..];
    let dl_request = HttpInV1 {
        method: request.method.clone(),
        path: dl_path.to_string(),
        query: request.query.clone(),
        headers: request.headers.clone(),
        body_b64: request.body_b64.clone(),
        route_hint: request.route_hint.clone(),
        binding_id: request.binding_id.clone(),
        config: request.config.clone(),
    };
    let mut state_driver = HostStateStore;
    // Use ConfigAwareSecretStore to check injected secrets from request config first,
    // falling back to host secrets_store interface if not found.
    let secrets_driver = ConfigAwareSecretStore::new(request.config.clone());
    let mut out = handle_directline_request_with_jwks(
        &dl_request,
        &mut state_driver,
        &secrets_driver,
        &HostJwksFetcher,
    );

    stamp_ingest_envelopes(request, dl_path, &mut out);

    http_out_v1_bytes(&out)
}

/// Whether conversation-create should emit the empty auto-start envelope.
///
/// A card-led flow absorbs that turn to render a welcome card; an
/// agent-forward flow (`start → dw.agent`) has nothing to absorb it and the
/// agent answers an empty message. Absent config keeps the historical
/// behaviour, so only an explicit `false` switches it off.
fn auto_start_on_open(config: Option<&Value>) -> bool {
    let Some(obj) = config.and_then(Value::as_object) else {
        return true;
    };
    if let Some(value) = obj.get("auto_start_on_open") {
        return bool_from_config_value(value).unwrap_or(true);
    }
    // The host may inject non-secret config fields base64-encoded.
    obj.get("auto_start_on_open_b64")
        .and_then(Value::as_str)
        .and_then(|encoded| general_purpose::STANDARD.decode(encoded.trim()).ok())
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .and_then(|text| text.trim().parse::<bool>().ok())
        .unwrap_or(true)
}

fn bool_from_config_value(value: &Value) -> Option<bool> {
    value
        .as_bool()
        .or_else(|| value.as_str().and_then(|s| s.trim().parse::<bool>().ok()))
}

/// Post-process the Direct Line response to emit `ChannelMessageEnvelope`
/// events for conversation creation and activity forwarding.
///
/// Separated from `handle_directline_path` so the envelope-stamping logic
/// (flow_hint, locale, metadata) can be tested without WASM host bindings.
fn stamp_ingest_envelopes(request: &HttpInV1, dl_path: &str, out: &mut HttpOutV1) {
    // Taken before anything else, and unconditionally, so the internal header
    // never reaches the client whichever branch below runs (or none does).
    let caller = take_caller_block(&mut out.headers);
    // Emit ChannelMessageEnvelope for POST /conversations so the operator can
    // auto-start the default flow when a new conversation is created.
    // The welcome experience is driven entirely by the flow — the JS-side
    // welcome overlay has been removed in favour of this server-side trigger.
    let conversation_created = request.method.eq_ignore_ascii_case("POST")
        && dl_path == "/v3/directline/conversations"
        && out.status == 201;
    let auto_start = auto_start_on_open(request.config.as_ref());
    if conversation_created && !auto_start {
        telemetry::emit(
            Level::Debug,
            PROVIDER_TYPE,
            "autoStart envelope suppressed by auto_start_on_open=false",
            &[],
        );
    }
    if conversation_created && auto_start {
        let (env_id, tenant_id) = extract_context_from_response_headers(&out.headers)
            .unwrap_or_else(|| ("default".to_string(), "default".to_string()));
        let user_id = out
            .headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case("X-Greentic-User"))
            .map(|h| h.value.clone());
        // Absent means the header wasn't set at all (shouldn't happen for a
        // 201 from handle_conversations) — treat that the same as "false" so
        // a missing flag can never be mistaken for a verified identity.
        let user_verified = out
            .headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case("X-Greentic-User-Verified"))
            .map(|h| h.value.trim().eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let conv_id = out
            .headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case("X-Greentic-ConversationId"))
            .map(|h| h.value.clone());
        let locale = request
            .headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case("X-Greentic-Locale"))
            .map(|h| h.value.trim().to_string())
            .filter(|v| !v.is_empty());
        let flow_hint = out
            .headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case("X-Greentic-Flow"))
            .map(|h| h.value.clone());
        // Surface the autoStart envelope shape so a missing welcome card on
        // the client can be diffed against the operator's flow execution.
        let conv_redacted = conv_id
            .as_deref()
            .map(redact::user_id)
            .unwrap_or_else(|| "<none>".to_string());
        let user_redacted = user_id
            .as_deref()
            .map(redact::user_id)
            .unwrap_or_else(|| "<none>".to_string());
        let locale_str = locale.as_deref().unwrap_or("<none>");
        let flow_str = flow_hint.as_deref().unwrap_or("<none>");
        telemetry::emit(
            Level::Debug,
            PROVIDER_TYPE,
            "autoStart envelope built",
            &[
                Field {
                    key: "env",
                    value: env_id.as_str(),
                },
                Field {
                    key: field::TENANT,
                    value: tenant_id.as_str(),
                },
                Field {
                    key: field::CONVERSATION_ID,
                    value: &conv_redacted,
                },
                Field {
                    key: field::USER,
                    value: &user_redacted,
                },
                Field {
                    key: "locale",
                    value: locale_str,
                },
                Field {
                    key: "flow_hint",
                    value: flow_str,
                },
            ],
        );
        let mut envelope = build_webchat_envelope_with_ctx(
            String::new(),
            user_id,
            conv_id,
            None,
            &env_id,
            &tenant_id,
            caller_extensions(BTreeMap::new(), caller.as_ref()),
        );
        let idempotency_key = user_entered_idempotency_key(
            "webchat",
            Some(&tenant_id),
            Some(&envelope.session_id),
            envelope.from.as_ref().map(|actor| actor.id.as_str()),
            "conversation_started",
        );
        mark_user_entered(
            &mut envelope.metadata,
            "webchat",
            "conversation_started",
            idempotency_key,
        );
        envelope
            .metadata
            .insert("tenant_id".to_string(), tenant_id.clone());
        envelope
            .metadata
            .insert("conversation_id".to_string(), envelope.session_id.clone());
        if let Some(actor) = &envelope.from {
            envelope
                .metadata
                .insert("user_id".to_string(), actor.id.clone());
        }
        // Unconditional, like the activities branch: an absent key would let a
        // consumer read "not applicable" where the answer is "not verified".
        debug_assert!(
            !user_verified || envelope.from.is_some(),
            "a verified flag with no subject would leave a consumer trusting a              client-supplied id for the missing user_id"
        );
        envelope
            .metadata
            .insert("user_verified".to_string(), user_verified.to_string());
        // Carry the picker locale through to the runner so the auto-start
        // welcome card is rendered in the user's language. Without this the
        // first card always renders in `en` because POST /conversations has
        // no activity body to read `locale` from. The SPA forwards the
        // selected locale via X-Greentic-Locale on the conversation-create
        // request; subsequent /activities POSTs already carry locale in the
        // BotFramework activity body.
        if let Some(locale) = locale {
            envelope.metadata.insert("locale".to_string(), locale);
        }
        if let Some(flow) = flow_hint {
            envelope.metadata.insert("flow_hint".to_string(), flow);
        }
        out.events.push(envelope);
    }

    // Emit ChannelMessageEnvelope for POST /activities so the operator can
    // forward user messages to the flow engine.
    if request.method.eq_ignore_ascii_case("POST")
        && dl_path.contains("/activities")
        && out.status == 201
    {
        let body = decode_body_json(&request.body_b64).unwrap_or(Value::Null);
        let text = extract_activity_text(&body);
        let action_value = body.get("value"); // Action.Submit data from AC buttons
        // The actor comes from the verified X-Greentic-User response header
        // (set by handle_post_activities from the JWT claims), never from the
        // request body's `from.id`, which is entirely client-chosen.
        let user = out
            .headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case("X-Greentic-User"))
            .map(|h| h.value.clone());
        let user_verified = out
            .headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case("X-Greentic-User-Verified"))
            .map(|h| h.value.trim().eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let conv_id = dl_path
            .strip_prefix("/v3/directline/conversations/")
            .and_then(|rest| rest.split('/').next())
            .map(|s| s.to_string());
        // Extract env and tenant from response headers (set by handle_post_activities)
        // This ensures we use the same context that was validated from the JWT
        let (env_id, tenant_id) = extract_context_from_response_headers(&out.headers)
            .unwrap_or_else(|| ("default".to_string(), "default".to_string()));
        // For Action.Submit, derive a text label from the action data.
        let effective_text = if text.is_empty() {
            action_value
                .and_then(|v| {
                    v.get("step")
                        .or(v.get("routeToCardId"))
                        .or(v.get("toCardId"))
                        .or(v.get("mcp_wizard"))
                        .or(v.get("mcp_operation"))
                })
                .and_then(|s| s.as_str())
                .unwrap_or("message")
                .to_string()
        } else {
            text
        };
        // The caller block is inserted AFTER the client-derived extensions, so
        // nothing in the activity body can pre-empt the verified one.
        let extensions = caller_extensions(collect_directline_extensions(&body), caller.as_ref());
        let mut envelope = build_webchat_envelope_with_ctx(
            effective_text,
            user,
            conv_id.clone(),
            None,
            &env_id,
            &tenant_id,
            extensions,
        );
        // Forward locale from the activity so the runner can resolve
        // i18n translations in card responses.
        if let Some(locale) = body.get("locale").and_then(Value::as_str)
            && !locale.is_empty()
        {
            envelope
                .metadata
                .insert("locale".to_string(), locale.to_string());
        }
        // Forward ALL Action.Submit data fields to metadata so the
        // operator can handle MCP actions, token saves, card routing, etc.
        // `user_id`/`user_verified` and any `user_*`/`greentic_*` key are
        // reserved for server-derived trust signals and are never taken
        // from client-supplied data — see the overwrite below.
        if let Some(val) = action_value
            && let Some(obj) = val.as_object()
        {
            for (k, v) in obj {
                if k.starts_with("user_") || k.starts_with("greentic_") {
                    continue;
                }
                let s = match v {
                    Value::String(s) => s.clone(),
                    _ => v.to_string(),
                };
                envelope.metadata.insert(k.clone(), s);
            }
        }
        // Stamp the server-verified trust signal after the Action.Submit
        // copy above so a client-supplied value can never survive it.
        if let Some(actor) = &envelope.from {
            envelope
                .metadata
                .insert("user_id".to_string(), actor.id.clone());
        }
        envelope
            .metadata
            .insert("user_verified".to_string(), user_verified.to_string());
        // Forward the persisted flow binding so every activity in the
        // conversation carries the same flow_hint the operator saw on
        // the auto-start envelope.
        if let Some(flow) = out
            .headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case("X-Greentic-Flow"))
            .map(|h| h.value.clone())
        {
            envelope.metadata.insert("flow_hint".to_string(), flow);
        }
        out.events.push(envelope);
    }
}

/// Remove the internal caller header from the Direct Line response and decode
/// it. `None` when absent or undecodable — the envelope then carries no
/// `extensions.caller`, which the runner reads as an anonymous caller.
fn take_caller_block(headers: &mut Vec<Header>) -> Option<Value> {
    let mut values = Vec::new();
    headers.retain(|h| {
        if h.name.eq_ignore_ascii_case(CALLER_HEADER) {
            values.push(h.value.clone());
            false
        } else {
            true
        }
    });
    // Exactly one: the handler emits a single header, so a second one means
    // something other than the handler wrote it, and neither can be trusted.
    match values.as_slice() {
        [value] => decode_caller_header(value),
        _ => None,
    }
}

/// `extensions` with the verified caller block stamped under `caller`,
/// overwriting any value already there. Without a block, an existing `caller`
/// key is REMOVED rather than kept: whatever put it there is not the token.
fn caller_extensions(
    mut extensions: BTreeMap<String, Value>,
    caller: Option<&Value>,
) -> BTreeMap<String, Value> {
    match caller {
        Some(block) => {
            extensions.insert(CALLER_EXT_KEY.to_string(), block.clone());
        }
        None => {
            extensions.remove(CALLER_EXT_KEY);
        }
    }
    extensions
}

/// Extract env and tenant from X-Greentic-Env and X-Greentic-Tenant response headers.
/// These headers are set by handle_post_activities after JWT validation.
fn extract_context_from_response_headers(headers: &[Header]) -> Option<(String, String)> {
    let env = headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case("X-Greentic-Env"))
        .map(|h| h.value.clone())?;
    let tenant = headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case("X-Greentic-Tenant"))
        .map(|h| h.value.clone())?;
    Some((env, tenant))
}

/// Collect DirectLine-native fields from an activity body into a typed
/// extensions map. Only fields that are present (and non-null) are inserted.
///
/// `channelData` is filtered to drop client-protocol fields that must NOT
/// echo back into the bot reply — `clientActivityID` and `postBack`. The
/// BotFramework-WebChat client uses `clientActivityID` to reconcile its
/// optimistic local bubble with the server echo; if the bot's outbound
/// activity carries the same ID, the client treats it as the user's own
/// echo and the bot card never renders.
fn collect_directline_extensions(body: &Value) -> BTreeMap<String, Value> {
    let mut ext = BTreeMap::new();

    if let Some(channel_data) = body.get("channelData").filter(|v| !v.is_null()) {
        let mut filtered = channel_data.clone();
        if let Some(obj) = filtered.as_object_mut() {
            obj.remove("clientActivityID");
            obj.remove("postBack");
        }
        let keep = filtered
            .as_object()
            .map(|obj| !obj.is_empty())
            .unwrap_or(true);
        if keep {
            ext.insert("channel_data".to_string(), filtered);
        }
    }

    let passthroughs: &[(&str, &str)] = &[
        ("attachments", "attachments"),
        ("entities", "entities"),
        ("name", "name"),
        ("inputHint", "input_hint"),
        ("speak", "speak"),
        ("suggestedActions", "suggested_actions"),
    ];
    for (src_key, ext_key) in passthroughs {
        if let Some(value) = body.get(*src_key)
            && !value.is_null()
        {
            ext.insert(ext_key.to_string(), value.clone());
        }
    }
    ext
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose;
    use serde_json::json;

    /// Build a minimal HttpInV1 for stamp_ingest_envelopes tests.
    fn build_ingest_request(
        method: &str,
        path: &str,
        headers: Vec<Header>,
        body: Option<&Value>,
    ) -> HttpInV1 {
        let body_b64 = body
            .map(|b| general_purpose::STANDARD.encode(serde_json::to_vec(b).unwrap()))
            .unwrap_or_default();
        HttpInV1 {
            method: method.to_string(),
            path: path.to_string(),
            query: None,
            headers,
            body_b64,
            route_hint: None,
            binding_id: None,
            config: None,
        }
    }

    /// Build a minimal HttpOutV1 that mimics a 201 Direct Line response.
    fn build_dl_response_201(headers: Vec<Header>) -> HttpOutV1 {
        HttpOutV1 {
            status: 201,
            headers,
            body_b64: String::new(),
            events: Vec::new(),
        }
    }

    fn conversation_create_headers() -> Vec<Header> {
        vec![
            Header {
                name: "X-Greentic-Env".into(),
                value: "prod".into(),
            },
            Header {
                name: "X-Greentic-Tenant".into(),
                value: "acme".into(),
            },
            Header {
                name: "X-Greentic-User".into(),
                value: "alice".into(),
            },
            Header {
                name: "X-Greentic-ConversationId".into(),
                value: "conv-1".into(),
            },
        ]
    }

    fn stamp_conversation_create(config: Option<Value>) -> HttpOutV1 {
        let mut request =
            build_ingest_request("POST", "/v3/directline/conversations", vec![], None);
        request.config = config;
        let mut out = build_dl_response_201(conversation_create_headers());
        stamp_ingest_envelopes(&request, "/v3/directline/conversations", &mut out);
        out
    }

    #[test]
    fn auto_start_envelope_is_emitted_when_the_flag_is_absent_or_true() {
        assert_eq!(stamp_conversation_create(None).events.len(), 1);
        assert_eq!(
            stamp_conversation_create(Some(json!({"auto_start_on_open": true})))
                .events
                .len(),
            1
        );
        assert_eq!(
            stamp_conversation_create(Some(json!({"auto_start_on_open": "true"})))
                .events
                .len(),
            1
        );
    }

    #[test]
    fn auto_start_envelope_is_suppressed_when_the_flag_is_false() {
        let out = stamp_conversation_create(Some(json!({"auto_start_on_open": false})));
        assert!(out.events.is_empty());
        assert_eq!(out.status, 201);

        assert!(
            stamp_conversation_create(Some(json!({"auto_start_on_open": "false"})))
                .events
                .is_empty()
        );

        let encoded = general_purpose::STANDARD.encode("false");
        assert!(
            stamp_conversation_create(Some(json!({"auto_start_on_open_b64": encoded})))
                .events
                .is_empty()
        );
    }

    #[test]
    fn a_suppressed_auto_start_still_forwards_user_activities() {
        let body = json!({"type": "message", "text": "hello", "from": {"id": "alice"}});
        let mut request = build_ingest_request(
            "POST",
            "/v3/directline/conversations/conv-1/activities",
            vec![],
            Some(&body),
        );
        request.config = Some(json!({"auto_start_on_open": false}));
        let mut out = build_dl_response_201(conversation_create_headers());
        stamp_ingest_envelopes(
            &request,
            "/v3/directline/conversations/conv-1/activities",
            &mut out,
        );
        assert_eq!(out.events.len(), 1);
        assert_eq!(out.events[0].text.as_deref(), Some("hello"));
    }

    #[test]
    fn conversation_envelope_carries_flow_hint_in_metadata() {
        let request = build_ingest_request("POST", "/v3/directline/conversations", vec![], None);
        let mut out = build_dl_response_201(vec![
            Header {
                name: "X-Greentic-Env".into(),
                value: "prod".into(),
            },
            Header {
                name: "X-Greentic-Tenant".into(),
                value: "acme".into(),
            },
            Header {
                name: "X-Greentic-User".into(),
                value: "alice".into(),
            },
            Header {
                name: "X-Greentic-ConversationId".into(),
                value: "conv-1".into(),
            },
            Header {
                name: "X-Greentic-Flow".into(),
                value: "onboarding-flow".into(),
            },
        ]);
        stamp_ingest_envelopes(&request, "/v3/directline/conversations", &mut out);

        assert_eq!(out.events.len(), 1);
        assert_eq!(
            out.events[0].metadata.get("flow_hint").map(String::as_str),
            Some("onboarding-flow"),
        );
    }

    #[test]
    fn conversation_envelope_omits_flow_hint_when_header_absent() {
        let request = build_ingest_request("POST", "/v3/directline/conversations", vec![], None);
        let mut out = build_dl_response_201(vec![
            Header {
                name: "X-Greentic-Env".into(),
                value: "prod".into(),
            },
            Header {
                name: "X-Greentic-Tenant".into(),
                value: "acme".into(),
            },
        ]);
        stamp_ingest_envelopes(&request, "/v3/directline/conversations", &mut out);

        assert_eq!(out.events.len(), 1);
        assert!(
            !out.events[0].metadata.contains_key("flow_hint"),
            "flow_hint must not appear when X-Greentic-Flow header is absent"
        );
    }

    #[test]
    fn conversation_envelope_always_carries_user_verified() {
        // A consumer reading `is_none()` as "not applicable" would treat an
        // anonymous auto-start as unguarded.
        let request = build_ingest_request("POST", "/v3/directline/conversations", vec![], None);
        let mut out = build_dl_response_201(vec![
            Header {
                name: "X-Greentic-Env".into(),
                value: "prod".into(),
            },
            Header {
                name: "X-Greentic-Tenant".into(),
                value: "acme".into(),
            },
        ]);
        stamp_ingest_envelopes(&request, "/v3/directline/conversations", &mut out);

        assert_eq!(out.events.len(), 1);
        assert_eq!(
            out.events[0]
                .metadata
                .get("user_verified")
                .map(String::as_str),
            Some("false"),
            "an auto-start envelope with no verified identity must say so explicitly"
        );
    }

    #[test]
    fn activity_envelope_carries_flow_hint_from_response_header() {
        let body = json!({
            "type": "message",
            "text": "hello",
            "from": {"id": "user-1"},
        });
        let request = build_ingest_request(
            "POST",
            "/v3/directline/conversations/conv-1/activities",
            vec![],
            Some(&body),
        );
        let mut out = build_dl_response_201(vec![
            Header {
                name: "X-Greentic-Env".into(),
                value: "prod".into(),
            },
            Header {
                name: "X-Greentic-Tenant".into(),
                value: "acme".into(),
            },
            Header {
                name: "X-Greentic-Flow".into(),
                value: "onboarding-flow".into(),
            },
        ]);
        stamp_ingest_envelopes(
            &request,
            "/v3/directline/conversations/conv-1/activities",
            &mut out,
        );

        assert_eq!(out.events.len(), 1);
        assert_eq!(
            out.events[0].metadata.get("flow_hint").map(String::as_str),
            Some("onboarding-flow"),
        );
    }

    #[test]
    fn activity_envelope_omits_flow_hint_when_header_absent() {
        let body = json!({
            "type": "message",
            "text": "hello",
            "from": {"id": "user-1"},
        });
        let request = build_ingest_request(
            "POST",
            "/v3/directline/conversations/conv-1/activities",
            vec![],
            Some(&body),
        );
        let mut out = build_dl_response_201(vec![
            Header {
                name: "X-Greentic-Env".into(),
                value: "prod".into(),
            },
            Header {
                name: "X-Greentic-Tenant".into(),
                value: "acme".into(),
            },
        ]);
        stamp_ingest_envelopes(
            &request,
            "/v3/directline/conversations/conv-1/activities",
            &mut out,
        );

        assert_eq!(out.events.len(), 1);
        assert!(
            !out.events[0].metadata.contains_key("flow_hint"),
            "flow_hint must not appear when X-Greentic-Flow header is absent"
        );
    }

    #[test]
    fn activity_envelope_takes_actor_from_header_not_body() {
        let body = json!({
            "type": "message",
            "text": "hello",
            "from": {"id": "attacker-supplied-id"},
        });
        let request = build_ingest_request(
            "POST",
            "/v3/directline/conversations/conv-1/activities",
            vec![],
            Some(&body),
        );
        let mut out = build_dl_response_201(vec![
            Header {
                name: "X-Greentic-Env".into(),
                value: "prod".into(),
            },
            Header {
                name: "X-Greentic-Tenant".into(),
                value: "acme".into(),
            },
            Header {
                name: "X-Greentic-User".into(),
                value: "verified-sub".into(),
            },
            Header {
                name: "X-Greentic-User-Verified".into(),
                value: "false".into(),
            },
        ]);
        stamp_ingest_envelopes(
            &request,
            "/v3/directline/conversations/conv-1/activities",
            &mut out,
        );

        assert_eq!(out.events.len(), 1);
        assert_eq!(
            out.events[0].from.as_ref().map(|a| a.id.as_str()),
            Some("verified-sub"),
            "actor must come from the X-Greentic-User response header, not body.from.id"
        );
    }

    // C1: a client posting a spoofed `user_verified`/`user_id` in the
    // Action.Submit value must never override the server-derived trust
    // signal, on an anonymous (unverified) DirectLine session.
    #[test]
    fn activity_envelope_rejects_spoofed_user_verified_from_action_submit_value() {
        let body = json!({
            "type": "message",
            "value": {"user_verified": "true", "user_id": "victim-sub"},
            "from": {"id": "victim-sub"},
        });
        let request = build_ingest_request(
            "POST",
            "/v3/directline/conversations/conv-1/activities",
            vec![],
            Some(&body),
        );
        let mut out = build_dl_response_201(vec![
            Header {
                name: "X-Greentic-Env".into(),
                value: "prod".into(),
            },
            Header {
                name: "X-Greentic-Tenant".into(),
                value: "acme".into(),
            },
            // Simulates handle_post_activities' output for an anonymous,
            // unverified DirectLine token.
            Header {
                name: "X-Greentic-User".into(),
                value: "guest-abc".into(),
            },
            Header {
                name: "X-Greentic-User-Verified".into(),
                value: "false".into(),
            },
        ]);
        stamp_ingest_envelopes(
            &request,
            "/v3/directline/conversations/conv-1/activities",
            &mut out,
        );

        assert_eq!(out.events.len(), 1);
        assert_eq!(
            out.events[0]
                .metadata
                .get("user_verified")
                .map(String::as_str),
            Some("false"),
            "a client-supplied user_verified must never survive into envelope metadata"
        );
        assert_eq!(
            out.events[0].metadata.get("user_id").map(String::as_str),
            Some("guest-abc"),
            "user_id metadata must be the server-verified actor, not the spoofed value"
        );
    }

    #[test]
    fn collect_directline_extensions_preserves_all_known_fields() {
        let body = json!({
            "type": "message",
            "text": "hi",
            "from": {"id": "user-1"},
            "attachments": [{
                "contentType": "application/vnd.microsoft.card.adaptive",
                "content": {"type": "AdaptiveCard", "body": []},
            }],
            "channelData": {"webchat": {"feature": "x"}, "rag": {"citations": [{"id": "c1"}]}},
            "entities": [{"type": "mention", "text": "@bot"}],
            "name": "event/custom",
            "inputHint": "acceptingInput",
            "speak": "hello there",
            "suggestedActions": {"actions": [{"type": "imBack", "title": "Yes", "value": "yes"}]},
        });

        let ext = collect_directline_extensions(&body);

        assert_eq!(
            ext.get("attachments"),
            Some(&json!([{
                "contentType": "application/vnd.microsoft.card.adaptive",
                "content": {"type": "AdaptiveCard", "body": []},
            }]))
        );
        assert_eq!(
            ext.get("channel_data"),
            Some(&json!({"webchat": {"feature": "x"}, "rag": {"citations": [{"id": "c1"}]}}))
        );
        assert_eq!(
            ext.get("entities"),
            Some(&json!([{"type": "mention", "text": "@bot"}]))
        );
        assert_eq!(ext.get("name"), Some(&json!("event/custom")));
        assert_eq!(ext.get("input_hint"), Some(&json!("acceptingInput")));
        assert_eq!(ext.get("speak"), Some(&json!("hello there")));
        assert_eq!(
            ext.get("suggested_actions"),
            Some(&json!({"actions": [{"type": "imBack", "title": "Yes", "value": "yes"}]}))
        );
    }

    #[test]
    fn collect_directline_extensions_omits_missing_fields() {
        let body = json!({"type": "message", "text": "hi"});
        let ext = collect_directline_extensions(&body);
        assert!(ext.is_empty());
    }

    #[test]
    fn collect_directline_extensions_omits_null_fields() {
        let body = json!({
            "type": "message",
            "attachments": null,
            "channelData": null,
        });
        let ext = collect_directline_extensions(&body);
        assert!(ext.is_empty());
    }

    #[test]
    fn collect_directline_extensions_strips_client_activity_id_from_channel_data() {
        // Action.Submit clicks include {clientActivityID, postBack} in channelData
        // for client-side reconciliation. If these leak into the bot's reply
        // channelData, BotFramework-WebChat treats the reply as an echo of the
        // user's POST and drops it — the bot card never renders.
        let body = json!({
            "type": "message",
            "from": {"id": "user-1"},
            "channelData": {
                "clientActivityID": "uqs9blhy9ph",
                "postBack": true,
                "webchat": {"feature": "keep-me"}
            }
        });

        let ext = collect_directline_extensions(&body);
        let channel_data = ext
            .get("channel_data")
            .expect("non-tracking fields preserved");
        assert!(channel_data.get("clientActivityID").is_none());
        assert!(channel_data.get("postBack").is_none());
        assert_eq!(channel_data["webchat"]["feature"], "keep-me");
    }

    #[test]
    fn collect_directline_extensions_drops_channel_data_when_only_tracking_fields_present() {
        let body = json!({
            "type": "message",
            "channelData": {
                "clientActivityID": "abc123",
                "postBack": true
            }
        });
        let ext = collect_directline_extensions(&body);
        assert!(
            !ext.contains_key("channel_data"),
            "channel_data containing only client tracking fields should be dropped"
        );
    }

    #[test]
    fn collect_directline_extensions_preserves_rag_citations_via_channel_data() {
        // Scenario from RAG component: citations smuggled in channelData.rag
        // (replacement for GTRC_RAG_BEGIN/END sentinel workaround).
        let body = json!({
            "type": "message",
            "text": "Based on your docs...",
            "from": {"id": "bot"},
            "channelData": {
                "rag": {
                    "type": "rag",
                    "citations": [
                        {"index": 1, "doc": "Metro Design v3.2", "source_file": "metro-design-v3.2.pdf"},
                        {"index": 2, "doc": "Capacity Review 2026", "source_file": "capacity-2026.pdf"}
                    ]
                }
            }
        });

        let ext = collect_directline_extensions(&body);
        let channel_data = ext.get("channel_data").expect("channel_data forwarded");
        let citations = channel_data
            .pointer("/rag/citations")
            .and_then(|v| v.as_array())
            .expect("citations preserved inside channel_data");
        assert_eq!(citations.len(), 2);
        assert_eq!(citations[0]["doc"], "Metro Design v3.2");
    }

    // --- extensions.caller: the verified caller greentic-runner#762 reads ---

    fn caller_header_value(block: &Value) -> String {
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(block).expect("json"))
    }

    fn activity_headers(extra: Vec<Header>) -> Vec<Header> {
        let mut headers = vec![
            Header {
                name: "X-Greentic-Env".into(),
                value: "prod".into(),
            },
            Header {
                name: "X-Greentic-Tenant".into(),
                value: "acme".into(),
            },
            Header {
                name: "X-Greentic-User".into(),
                value: "acme:users:7".into(),
            },
            Header {
                name: "X-Greentic-User-Verified".into(),
                value: "true".into(),
            },
        ];
        headers.extend(extra);
        headers
    }

    fn stamp_activity(body: &Value, headers: Vec<Header>) -> HttpOutV1 {
        let path = "/v3/directline/conversations/conv-1/activities";
        let request = build_ingest_request("POST", path, vec![], Some(body));
        let mut out = build_dl_response_201(headers);
        stamp_ingest_envelopes(&request, path, &mut out);
        out
    }

    #[test]
    fn activity_envelope_stamps_the_verified_caller_and_hides_the_header() {
        let block = json!({
            "user_verified": true, "sub": "acme:users:7", "team": "ops",
            "groups": ["hr"], "role": "manager",
        });
        let out = stamp_activity(
            &json!({"type": "message", "text": "hi"}),
            activity_headers(vec![Header {
                name: CALLER_HEADER.into(),
                value: caller_header_value(&block),
            }]),
        );
        assert_eq!(out.events.len(), 1);
        assert_eq!(out.events[0].extensions.get("caller"), Some(&block));
        assert!(
            out.headers
                .iter()
                .all(|h| !h.name.eq_ignore_ascii_case(CALLER_HEADER)),
            "the internal caller header must not leave the provider"
        );
        // The serialized envelope is what greentic-runner reads.
        let payload = serde_json::to_value(&out.events[0]).expect("serialize");
        assert_eq!(payload["extensions"]["caller"], block);
    }

    #[test]
    fn conversation_envelope_stamps_the_verified_caller() {
        let block = json!({"user_verified": true, "sub": "alice"});
        let request = build_ingest_request("POST", "/v3/directline/conversations", vec![], None);
        let mut headers = conversation_create_headers();
        headers.push(Header {
            name: CALLER_HEADER.into(),
            value: caller_header_value(&block),
        });
        let mut out = build_dl_response_201(headers);
        stamp_ingest_envelopes(&request, "/v3/directline/conversations", &mut out);
        assert_eq!(out.events.len(), 1);
        assert_eq!(out.events[0].extensions.get("caller"), Some(&block));
    }

    #[test]
    fn without_a_caller_header_no_caller_is_stamped() {
        let out = stamp_activity(
            &json!({"type": "message", "text": "hi"}),
            activity_headers(vec![]),
        );
        assert_eq!(out.events.len(), 1);
        assert!(!out.events[0].extensions.contains_key("caller"));
    }

    #[test]
    fn a_client_supplied_caller_never_reaches_the_envelope() {
        // Neither channelData nor Action.Submit data nor a request header can
        // establish a caller; only the handler's response header can.
        let body = json!({
            "type": "message",
            "text": "hi",
            "channelData": {"caller": {"user_verified": true, "groups": ["admins"]}},
            "value": {"caller": "{\"user_verified\":true}"},
        });
        let path = "/v3/directline/conversations/conv-1/activities";
        let request = build_ingest_request(
            "POST",
            path,
            vec![Header {
                name: CALLER_HEADER.into(),
                value: caller_header_value(&json!({"user_verified": true, "groups": ["admins"]})),
            }],
            Some(&body),
        );
        let mut out = build_dl_response_201(activity_headers(vec![]));
        stamp_ingest_envelopes(&request, path, &mut out);
        assert_eq!(out.events.len(), 1);
        assert!(!out.events[0].extensions.contains_key("caller"));
    }

    #[test]
    fn malformed_or_duplicated_caller_headers_stamp_nothing() {
        let body = json!({"type": "message", "text": "hi"});
        let malformed = stamp_activity(
            &body,
            activity_headers(vec![Header {
                name: CALLER_HEADER.into(),
                value: "not-base64!".into(),
            }]),
        );
        assert!(!malformed.events[0].extensions.contains_key("caller"));
        assert!(
            malformed
                .headers
                .iter()
                .all(|h| !h.name.eq_ignore_ascii_case(CALLER_HEADER))
        );

        let one = caller_header_value(&json!({"user_verified": true, "sub": "a"}));
        let two = caller_header_value(&json!({"user_verified": true, "sub": "b"}));
        let duplicated = stamp_activity(
            &body,
            activity_headers(vec![
                Header {
                    name: CALLER_HEADER.into(),
                    value: one,
                },
                Header {
                    name: CALLER_HEADER.to_lowercase(),
                    value: two,
                },
            ]),
        );
        assert!(!duplicated.events[0].extensions.contains_key("caller"));
    }
}
