//! Inbound webhook ingestion from Bot Framework.
//!
//! `ingest_http` is invoked by the operator HTTP adapter for every Bot
//! Framework activity: it validates the bearer token (Phase 1: decode-only),
//! extracts `serviceUrl`, `conversation.id`, and `activity.id` for downstream
//! replies, and converts Bot Framework activities (including Adaptive Card
//! actions) into `ChannelMessageEnvelope`s for the flow runtime.

use base64::{Engine, engine::general_purpose::STANDARD};
use greentic_types::messaging::universal_dto::{HttpInV1, HttpOutV1};
use provider_common::http_compat::{http_out_error, http_out_v1_bytes, parse_operator_http_in};
use provider_common::redact;
use provider_common::telemetry::{self, Field, Level, event, field};
use serde_json::{Value, json};

use crate::PROVIDER_TYPE;
use crate::auth::{extract_bearer_token, validate_jwt};
use crate::config::{get_activity_id, get_conversation_id, load_config};

use super::build_team_envelope;

/// Handles incoming HTTP webhook from Bot Framework.
///
/// - Validates JWT from Authorization header
/// - Parses Bot Framework Activity
/// - Extracts serviceUrl, conversation.id, activity.id for downstream replies
pub(crate) fn ingest_http(input_json: &[u8]) -> Vec<u8> {
    // Try native greentic-types format first, fall back to operator format
    let request = match serde_json::from_slice::<HttpInV1>(input_json) {
        Ok(req) => req,
        Err(_) => match parse_operator_http_in(input_json) {
            Ok(req) => req,
            Err(err) => return http_out_error(400, &format!("invalid http input: {err}")),
        },
    };

    // Load config for JWT validation and skip_jwt_validation check
    let parsed_input: Value = serde_json::from_slice(input_json).unwrap_or(Value::Null);
    let cfg = match load_config(&parsed_input) {
        Ok(c) => c,
        Err(_) => return http_out_error(500, "provider config error"),
    };

    if cfg.skip_jwt_validation == Some(true) {
        telemetry::emit(
            Level::Warn,
            PROVIDER_TYPE,
            "skip_jwt_validation=true, JWT checks bypassed (dev only)",
            &[],
        );
    } else {
        let app_id = match cfg.ms_bot_app_id.as_deref() {
            Some(id) if !id.is_empty() => id,
            _ => return http_out_error(401, "ms_bot_app_id not configured"),
        };
        let auth_header = request
            .headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case("authorization"))
            .map(|h| h.value.as_str());
        let auth = match auth_header {
            Some(h) => h,
            None => return http_out_error(401, "missing Authorization header"),
        };
        let token = match extract_bearer_token(auth) {
            Some(t) => t,
            None => return http_out_error(401, "invalid Authorization scheme"),
        };
        if let Err(err) = validate_jwt(&token, app_id) {
            let detail = redact::error_message(&err);
            telemetry::emit(
                Level::Warn,
                PROVIDER_TYPE,
                "JWT validation rejected",
                &[
                    Field {
                        key: field::EVENT_KIND,
                        value: event::WEBHOOK_REJECTED,
                    },
                    Field {
                        key: field::ERROR,
                        value: &detail,
                    },
                ],
            );
            return http_out_error(401, "JWT validation failed");
        }
    }

    let body_bytes = match STANDARD.decode(&request.body_b64) {
        Ok(bytes) => bytes,
        Err(err) => return http_out_error(400, &format!("invalid body encoding: {err}")),
    };
    let body_val: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);

    // Bot Framework activity: detect type and handle accordingly
    let activity_type = body_val
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();

    // Extract critical fields for downstream operations
    let service_url = body_val
        .get("serviceUrl")
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    let conversation_id = get_conversation_id(&body_val);
    let activity_id = get_activity_id(&body_val);

    let is_bot_framework = activity_type == "invoke" || activity_type == "message";
    let is_card_action = is_bot_framework
        && (body_val.get("value").is_some()
            || body_val
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|n| n.starts_with("adaptiveCard/")));

    if is_card_action {
        return handle_card_action(&body_val, service_url, conversation_id, activity_id);
    }

    // Bot Framework message activity (regular text messages)
    let text = extract_bot_text(&body_val);
    let team_id = extract_team_id(&body_val);
    let channel_id = extract_channel_id(&body_val);
    let user = extract_sender(&body_val);

    let mut envelope = build_team_envelope(text.clone(), user, team_id.clone(), channel_id.clone());

    // Forward locale from Bot Framework activity for i18n card translation.
    if let Some(locale) = body_val.get("locale").and_then(Value::as_str)
        && !locale.is_empty()
    {
        envelope
            .metadata
            .insert("locale".to_string(), locale.to_string());
    }

    // Store critical fields for downstream replies
    if let Some(ref url) = service_url {
        envelope.metadata.insert("serviceUrl".into(), url.clone());
    }
    if let Some(ref conv_id) = conversation_id {
        envelope
            .metadata
            .insert("conversationId".into(), conv_id.clone());
    }
    if let Some(ref act_id) = activity_id {
        envelope
            .metadata
            .insert("activityId".into(), act_id.clone());
        envelope
            .metadata
            .insert("reply_to_id".into(), act_id.clone());
    }

    let normalized = json!({
        "ok": true,
        "event": body_val,
        "team_id": team_id,
        "channel_id": channel_id,
        "service_url": service_url,
        "conversation_id": conversation_id,
        "activity_id": activity_id,
    });
    let normalized_bytes = serde_json::to_vec(&normalized).unwrap_or_else(|_| b"{}".to_vec());
    let out = HttpOutV1 {
        status: 200,
        headers: Vec::new(),
        body_b64: STANDARD.encode(&normalized_bytes),
        events: vec![envelope],
    };
    http_out_v1_bytes(&out)
}

/// Converts a Bot Framework `invoke` / `message` activity carrying an Adaptive
/// Card action (`Action.Submit` or `Action.Execute`) into a synthetic envelope
/// with all action fields forwarded as metadata for MCP routing.
fn handle_card_action(
    body_val: &Value,
    service_url: Option<String>,
    conversation_id: Option<String>,
    activity_id: Option<String>,
) -> Vec<u8> {
    let sender = body_val
        .get("from")
        .and_then(|f| f.get("id"))
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    let team_id = body_val
        .get("channelData")
        .and_then(|cd| cd.get("team"))
        .and_then(|t| t.get("id"))
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    let channel_id = body_val
        .get("channelData")
        .and_then(|cd| cd.get("channel"))
        .and_then(|c| c.get("id"))
        .and_then(Value::as_str)
        .map(|s| s.to_string());

    // For Action.Execute: value.action.data
    // For Action.Submit: value directly
    let action_data = body_val
        .get("value")
        .and_then(|v| v.get("action"))
        .and_then(|a| a.get("data"))
        .cloned()
        .or_else(|| body_val.get("value").cloned())
        .unwrap_or(Value::Null);

    let route_to_card = action_data
        .get("routeToCardId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let card_id = action_data
        .get("cardId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let action_text = if !route_to_card.is_empty() {
        format!("[card:{route_to_card}]")
    } else if !card_id.is_empty() {
        format!("[action:{card_id}]")
    } else {
        "[card-action]".to_string()
    };

    let mut envelope =
        build_team_envelope(action_text, sender, team_id.clone(), channel_id.clone());

    // Store critical fields for downstream replies
    if let Some(ref url) = service_url {
        envelope.metadata.insert("serviceUrl".into(), url.clone());
    }
    if let Some(ref conv_id) = conversation_id {
        envelope
            .metadata
            .insert("conversationId".into(), conv_id.clone());
    }
    if let Some(ref act_id) = activity_id {
        envelope
            .metadata
            .insert("activityId".into(), act_id.clone());
        envelope
            .metadata
            .insert("reply_to_id".into(), act_id.clone());
    }

    if !route_to_card.is_empty() {
        envelope
            .metadata
            .insert("routeToCardId".into(), route_to_card);
    }
    if !card_id.is_empty() {
        envelope.metadata.insert("cardId".into(), card_id);
    }
    // Forward ALL action_data fields to metadata for MCP routing
    if let Some(obj) = action_data.as_object() {
        for (k, v) in obj {
            let s = match v {
                Value::String(s) => s.clone(),
                _ => v.to_string(),
            };
            envelope.metadata.insert(k.clone(), s);
        }
    }
    envelope.metadata.insert(
        "teams.actionData".into(),
        serde_json::to_string(&action_data).unwrap_or_default(),
    );

    let normalized = json!({
        "ok": true,
        "event": body_val,
        "team_id": team_id,
        "channel_id": channel_id,
        "service_url": service_url,
        "conversation_id": conversation_id,
        "activity_id": activity_id,
    });
    let normalized_bytes = serde_json::to_vec(&normalized).unwrap_or_else(|_| b"{}".to_vec());
    let out = HttpOutV1 {
        status: 200,
        headers: Vec::new(),
        body_b64: STANDARD.encode(&normalized_bytes),
        events: vec![envelope],
    };
    http_out_v1_bytes(&out)
}

/// Extracts message text from Bot Framework Activity.
pub(crate) fn extract_bot_text(value: &Value) -> String {
    // Bot Framework Activity: text is at top level
    value
        .get("text")
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .unwrap_or_default()
}

pub(crate) fn extract_team_id(value: &Value) -> Option<String> {
    // Bot Framework: channelData.team.id
    value
        .get("channelData")
        .and_then(|cd| cd.get("team"))
        .and_then(|t| t.get("id"))
        .and_then(Value::as_str)
        .map(|s| s.to_string())
}

pub(crate) fn extract_channel_id(value: &Value) -> Option<String> {
    // Bot Framework: channelData.channel.id
    value
        .get("channelData")
        .and_then(|cd| cd.get("channel"))
        .and_then(|c| c.get("id"))
        .and_then(Value::as_str)
        .map(|s| s.to_string())
}

pub(crate) fn extract_sender(value: &Value) -> Option<String> {
    // Bot Framework: from.id
    value
        .get("from")
        .and_then(|f| f.get("id"))
        .and_then(Value::as_str)
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::fake_signed_token;
    use base64::{Engine, engine::general_purpose::STANDARD};
    use greentic_types::messaging::universal_dto::{Header, HttpInV1};

    fn valid_token(app_id: &str) -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        fake_signed_token(json!({
            "iss": "https://api.botframework.com",
            "aud": app_id,
            "exp": now + 3600,
        }))
    }

    fn bot_activity_body() -> String {
        STANDARD.encode(
            serde_json::to_vec(&json!({
                "type": "message",
                "text": "hello",
                "from": { "id": "user-1" },
                "conversation": { "id": "conv-1" },
                "id": "act-1",
                "serviceUrl": "https://smba.trafficmanager.net/amer/"
            }))
            .unwrap(),
        )
    }

    fn build_ingest_input(
        auth_header: Option<&str>,
        app_id: Option<&str>,
        skip_jwt: Option<bool>,
    ) -> Vec<u8> {
        // graph mode: bot_framework setup_mode is rejected by validate_provider_config,
        // so ingest JWT tests must use a loadable graph config carrying ms_bot_app_id.
        build_ingest_input_with_mode(auth_header, app_id, skip_jwt, false)
    }

    fn build_ingest_input_with_mode(
        auth_header: Option<&str>,
        app_id: Option<&str>,
        skip_jwt: Option<bool>,
        bot_framework_mode: bool,
    ) -> Vec<u8> {
        let body_b64 = bot_activity_body();
        let mut headers = vec![];
        if let Some(auth) = auth_header {
            headers.push(Header {
                name: "Authorization".to_string(),
                value: auth.to_string(),
            });
        }

        let http_in = HttpInV1 {
            method: "POST".to_string(),
            path: "/api/messages".to_string(),
            query: None,
            headers,
            body_b64,
            route_hint: None,
            binding_id: None,
            config: None,
        };
        let mut val = serde_json::to_value(&http_in).unwrap();
        let obj = val.as_object_mut().unwrap();

        // Build a nested config object so load_config picks it up via input.get("config").
        let mut cfg = serde_json::Map::new();
        if bot_framework_mode {
            cfg.insert("setup_mode".into(), json!("bot_framework"));
        } else {
            cfg.insert("tenant_id".into(), json!("tenant"));
            cfg.insert("client_id".into(), json!("client"));
        }
        if let Some(id) = app_id {
            cfg.insert("ms_bot_app_id".into(), json!(id));
        }
        if let Some(skip) = skip_jwt {
            cfg.insert("skip_jwt_validation".into(), json!(skip));
        }
        obj.insert("config".into(), Value::Object(cfg));
        serde_json::to_vec(&val).unwrap()
    }

    fn parse_response(bytes: Vec<u8>) -> (u16, Value) {
        let out: Value = serde_json::from_slice(&bytes).unwrap();
        let status = out["status"].as_u64().unwrap() as u16;
        (status, out)
    }

    #[test]
    fn ingest_missing_auth_header_rejects_401() {
        let input = build_ingest_input(None, Some("bot-app-id"), None);
        let (status, _) = parse_response(ingest_http(&input));
        assert_eq!(status, 401);
    }

    #[test]
    fn ingest_non_bearer_scheme_rejects_401() {
        let input = build_ingest_input(Some("Basic abc123"), Some("bot-app-id"), None);
        let (status, _) = parse_response(ingest_http(&input));
        assert_eq!(status, 401);
    }

    #[test]
    fn ingest_missing_app_id_rejects_401() {
        let token = valid_token("bot-app-id");
        // Use graph mode so config loads successfully but ms_bot_app_id is absent.
        let input =
            build_ingest_input_with_mode(Some(&format!("Bearer {token}")), None, None, false);
        let (status, _) = parse_response(ingest_http(&input));
        assert_eq!(status, 401);
    }

    #[test]
    fn ingest_malformed_jwt_rejects_401() {
        let input = build_ingest_input(Some("Bearer not-a-jwt"), Some("bot-app-id"), None);
        let (status, _) = parse_response(ingest_http(&input));
        assert_eq!(status, 401);
    }

    #[test]
    fn ingest_wrong_audience_rejects_401() {
        let token = valid_token("wrong-app-id");
        let input = build_ingest_input(Some(&format!("Bearer {token}")), Some("bot-app-id"), None);
        let (status, _) = parse_response(ingest_http(&input));
        assert_eq!(status, 401);
    }

    #[test]
    fn ingest_expired_token_rejects_401() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let token = fake_signed_token(json!({
            "iss": "https://api.botframework.com",
            "aud": "bot-app-id",
            "exp": now - crate::auth::CLOCK_SKEW_SECS - 1,
        }));
        let input = build_ingest_input(Some(&format!("Bearer {token}")), Some("bot-app-id"), None);
        let (status, _) = parse_response(ingest_http(&input));
        assert_eq!(status, 401);
    }

    #[test]
    fn ingest_expired_within_skew_accepted() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let token = fake_signed_token(json!({
            "iss": "https://api.botframework.com",
            "aud": "bot-app-id",
            "exp": now - 60,
        }));
        let input = build_ingest_input(Some(&format!("Bearer {token}")), Some("bot-app-id"), None);
        let (status, _) = parse_response(ingest_http(&input));
        assert_eq!(status, 200);
    }

    #[test]
    fn ingest_invalid_issuer_rejects_401() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let token = fake_signed_token(json!({
            "iss": "https://evil.example.com",
            "aud": "bot-app-id",
            "exp": now + 3600,
        }));
        let input = build_ingest_input(Some(&format!("Bearer {token}")), Some("bot-app-id"), None);
        let (status, _) = parse_response(ingest_http(&input));
        assert_eq!(status, 401);
    }

    #[test]
    fn ingest_of_a_bot_activity_with_a_file_emits_no_attachments_in_v1() {
        // The Graph provider's own Bot Framework ingest is deferred in v1: a
        // Teams file upload or inline image must not reach the envelope here.
        let token = valid_token("bot-app-id");
        let input = build_ingest_input(Some(&format!("Bearer {token}")), Some("bot-app-id"), None);
        let mut input: Value = serde_json::from_slice(&input).unwrap();
        input["body_b64"] = json!(STANDARD.encode(
            serde_json::to_vec(&json!({
                "type": "message",
                "text": "here",
                "from": { "id": "user-1" },
                "conversation": { "id": "conv-1" },
                "id": "act-1",
                "serviceUrl": "https://smba.trafficmanager.net/amer/",
                "attachments": [
                    {"contentType": "application/vnd.microsoft.teams.file.download.info",
                     "name": "plan.pdf",
                     "content": {"downloadUrl": "https://contoso.sharepoint.com/d", "fileType": "pdf"}},
                    {"contentType": "image/png",
                     "contentUrl": "https://smba.trafficmanager.net/amer/v3/attachments/1"}
                ]
            }))
            .unwrap()
        ));
        let (status, body) = parse_response(ingest_http(&serde_json::to_vec(&input).unwrap()));
        assert_eq!(status, 200);
        let event = &body["events"][0];
        assert_eq!(event["text"], "here");
        assert!(event["attachments"].as_array().is_none_or(Vec::is_empty));
        assert!(event["extensions"].get("attachment_fetch").is_none());
        assert!(event["extensions"].get("artifacts").is_none());
    }

    #[test]
    fn ingest_valid_token_accepts_200() {
        let token = valid_token("bot-app-id");
        let input = build_ingest_input(Some(&format!("Bearer {token}")), Some("bot-app-id"), None);
        let (status, body) = parse_response(ingest_http(&input));
        assert_eq!(status, 200);
        assert!(!body["events"].as_array().unwrap().is_empty());
    }

    /// Env-var gate for skip_jwt_validation is unit-tested in config::tests
    /// (skip_jwt_validation_without_env_var_rejected / _with_env_var_accepted).
    /// Integration-level ingest test exercises the happy path only to avoid
    /// env-var races under parallel test execution.
    #[test]
    fn ingest_skip_validation_with_env_var_accepts_invalid() {
        // SAFETY: test-only; tests that touch this env var must run serialized
        // or accept the race. The config-level tests cover the rejection path.
        unsafe { std::env::set_var("GREENTIC_TEAMS_INSECURE_DEV", "1") };
        let input = build_ingest_input(Some("Bearer not-a-jwt"), Some("bot-app-id"), Some(true));
        let (status, _) = parse_response(ingest_http(&input));
        assert_eq!(status, 200);
        unsafe { std::env::remove_var("GREENTIC_TEAMS_INSECURE_DEV") };
    }
}
