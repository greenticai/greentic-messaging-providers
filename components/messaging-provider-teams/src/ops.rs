//! Core operations for Teams Bot Service provider.
//!
//! Uses Bot Connector API instead of Microsoft Graph API.

use base64::{Engine, engine::general_purpose::STANDARD};
use greentic_types::messaging::universal_dto::{
    HttpInV1, HttpOutV1, ProviderPayloadV1, SendPayloadInV1,
};
use greentic_types::{
    Actor, ChannelMessageEnvelope, Destination, EnvId, MessageMetadata, TenantCtx, TenantId,
};
use provider_common::helpers::{
    PlannerCapabilities, RenderPlanConfig, decode_encode_message, encode_error, json_bytes,
    render_plan_common, send_payload_error,
};
use provider_common::http_compat::{http_out_error, http_out_v1_bytes, parse_operator_http_in};
use serde_json::{Value, json};
use std::collections::BTreeMap;

use crate::PROVIDER_TYPE;
use crate::auth::{acquire_bot_token, extract_bearer_token, validate_jwt};
use crate::bindings::greentic::http::http_client as client;
use crate::config::{
    ProviderConfig, default_channel_destination, get_activity_id, get_conversation_id,
    get_service_url, load_config,
};

/// Handles the "send" operation - sends a message via Bot Connector API.
///
/// Endpoint: `{serviceUrl}/v3/conversations/{conversationId}/activities`
pub(crate) fn handle_send(input_json: &[u8]) -> Vec<u8> {
    let parsed: Value = match serde_json::from_slice(input_json) {
        Ok(val) => val,
        Err(err) => {
            return json_bytes(&json!({"ok": false, "error": format!("invalid json: {err}") }));
        }
    };

    let cfg = match load_config(&parsed) {
        Ok(cfg) => cfg,
        Err(err) => return json_bytes(&json!({"ok": false, "error": err})),
    };
    if !cfg.enabled {
        return json_bytes(&json!({"ok": false, "error": "provider disabled by config"}));
    }

    let envelope: ChannelMessageEnvelope = match serde_json::from_slice(input_json) {
        Ok(env) => env,
        Err(err) => match build_team_envelope_from_input(&parsed, &cfg) {
            Ok(env) => env,
            Err(message) => {
                return json_bytes(
                    &json!({"ok": false, "error": format!("invalid envelope: {message}: {err}") }),
                );
            }
        },
    };

    if !envelope.attachments.is_empty() {
        return json_bytes(&json!({"ok": false, "error": "attachments not supported"}));
    }

    let text = envelope
        .text
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let text = match text {
        Some(value) => value,
        None => return json_bytes(&json!({"ok": false, "error": "text required"})),
    };

    // Get service URL from metadata or config
    let service_url = parsed
        .get("metadata")
        .and_then(|m| m.get("serviceUrl"))
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .or_else(|| get_service_url(&parsed, &cfg))
        .or_else(|| cfg.default_service_url.clone());

    let service_url = match service_url {
        Some(url) if !url.is_empty() => url.trim_end_matches('/').to_string(),
        _ => {
            return json_bytes(&json!({
                "ok": false,
                "error": "service_url required (from metadata.serviceUrl, config.default_service_url, or Activity)"
            }));
        }
    };

    // Get conversation ID from metadata or destination
    let conversation_id = parsed
        .get("metadata")
        .and_then(|m| m.get("conversationId"))
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .or_else(|| {
            envelope
                .to
                .first()
                .map(|d| d.id.clone())
                .or_else(|| default_channel_destination(&cfg).map(|d| d.id))
        });

    let conversation_id = match conversation_id {
        Some(id) if !id.is_empty() => id,
        _ => {
            return json_bytes(&json!({
                "ok": false,
                "error": "conversation_id required (from metadata.conversationId or destination)"
            }));
        }
    };

    // Reply-in-thread: check for reply_to_id in envelope or metadata
    let reply_to_id = parsed
        .get("reply_to_id")
        .and_then(Value::as_str)
        .or_else(|| parsed.get("reply_scope").and_then(Value::as_str))
        .or_else(|| {
            parsed
                .get("metadata")
                .and_then(|m| m.get("reply_to_id"))
                .and_then(Value::as_str)
        })
        .map(|s| s.trim_matches('"'))
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    // Acquire bot token
    let token = match acquire_bot_token(&cfg) {
        Ok(tok) => tok,
        Err(err) => return json_bytes(&json!({"ok": false, "error": err})),
    };

    // Build Bot Framework Activity payload
    let ac_json_str = parsed
        .get("_ac_json")
        .and_then(Value::as_str)
        .and_then(|s| serde_json::from_str::<Value>(s).ok());

    let body = if let Some(ac_card) = ac_json_str {
        // Send as native Adaptive Card attachment
        json!({
            "type": "message",
            "text": "",
            "attachments": [{
                "contentType": "application/vnd.microsoft.card.adaptive",
                "content": ac_card
            }]
        })
    } else {
        json!({
            "type": "message",
            "text": text
        })
    };

    // Build URL for Bot Connector API
    let url = if let Some(ref activity_id) = reply_to_id {
        format!(
            "{}/v3/conversations/{}/activities/{}",
            service_url, conversation_id, activity_id
        )
    } else {
        format!(
            "{}/v3/conversations/{}/activities",
            service_url, conversation_id
        )
    };

    let request = client::Request {
        method: "POST".into(),
        url,
        headers: vec![
            ("Content-Type".into(), "application/json".into()),
            ("Authorization".into(), format!("Bearer {token}")),
        ],
        body: Some(serde_json::to_vec(&body).unwrap_or_else(|_| b"{}".to_vec())),
    };

    let resp = match client::send(&request, None, None) {
        Ok(resp) => resp,
        Err(err) => {
            return json_bytes(&json!({
                "ok": false,
                "error": format!("transport error: {}", err.message),
            }));
        }
    };

    if resp.status < 200 || resp.status >= 300 {
        let err_body = resp
            .body
            .as_ref()
            .and_then(|b| serde_json::from_slice::<Value>(b).ok())
            .unwrap_or(Value::Null);
        let err_msg = err_body
            .get("message")
            .or_else(|| err_body.get("error").and_then(|e| e.get("message")))
            .and_then(Value::as_str)
            .unwrap_or("");
        return json_bytes(&json!({
            "ok": false,
            "error": format!("bot connector returned status {}: {}", resp.status, err_msg),
            "response": err_body,
        }));
    }

    let body_bytes = resp.body.unwrap_or_default();
    let body_json: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    let message_id = body_json
        .get("id")
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .unwrap_or_else(|| "bot-message".to_string());
    let provider_message_id = format!("teams:{message_id}");

    json_bytes(&json!({
        "ok": true,
        "status": "sent",
        "provider_type": PROVIDER_TYPE,
        "public_base_url": cfg.public_base_url,
        "message_id": message_id,
        "provider_message_id": provider_message_id,
        "response": body_json,
    }))
}

/// Handles the "reply" operation - replies in a thread via Bot Connector API.
///
/// Endpoint: `{serviceUrl}/v3/conversations/{conversationId}/activities/{replyToId}`
pub(crate) fn handle_reply(input_json: &[u8]) -> Vec<u8> {
    let parsed: Value = match serde_json::from_slice(input_json) {
        Ok(val) => val,
        Err(err) => {
            return json_bytes(&json!({"ok": false, "error": format!("invalid json: {err}")}));
        }
    };

    let cfg = match load_config(&parsed) {
        Ok(cfg) => cfg,
        Err(err) => return json_bytes(&json!({"ok": false, "error": err})),
    };
    if !cfg.enabled {
        return json_bytes(&json!({"ok": false, "error": "provider disabled by config"}));
    }

    let reply_to_id = parsed
        .get("reply_to_id")
        .or_else(|| parsed.get("thread_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if reply_to_id.is_empty() {
        return json_bytes(&json!({"ok": false, "error": "reply_to_id or thread_id required"}));
    }

    let text = parsed
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if text.is_empty() {
        return json_bytes(&json!({"ok": false, "error": "text required"}));
    }

    // Get service URL
    let service_url = parsed
        .get("service_url")
        .or_else(|| parsed.get("serviceUrl"))
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .or_else(|| cfg.default_service_url.clone());

    let service_url = match service_url {
        Some(url) if !url.is_empty() => url.trim_end_matches('/').to_string(),
        _ => {
            return json_bytes(&json!({
                "ok": false,
                "error": "service_url required"
            }));
        }
    };

    // Get conversation ID
    let conversation_id = parsed
        .get("conversation_id")
        .or_else(|| parsed.get("conversationId"))
        .and_then(Value::as_str)
        .map(|s| s.to_string());

    let conversation_id = match conversation_id {
        Some(id) if !id.is_empty() => id,
        _ => {
            return json_bytes(&json!({
                "ok": false,
                "error": "conversation_id required"
            }));
        }
    };

    let token = match acquire_bot_token(&cfg) {
        Ok(tok) => tok,
        Err(err) => return json_bytes(&json!({"ok": false, "error": err})),
    };

    let url = format!(
        "{}/v3/conversations/{}/activities/{}",
        service_url, conversation_id, reply_to_id
    );

    let body = json!({
        "type": "message",
        "text": text
    });

    let request = client::Request {
        method: "POST".into(),
        url,
        headers: vec![
            ("Content-Type".into(), "application/json".into()),
            ("Authorization".into(), format!("Bearer {token}")),
        ],
        body: Some(serde_json::to_vec(&body).unwrap_or_else(|_| b"{}".to_vec())),
    };

    let resp = match client::send(&request, None, None) {
        Ok(resp) => resp,
        Err(err) => {
            return json_bytes(&json!({
                "ok": false,
                "error": format!("transport error: {}", err.message),
            }));
        }
    };

    if resp.status < 200 || resp.status >= 300 {
        return json_bytes(&json!({
            "ok": false,
            "error": format!("bot connector returned status {}", resp.status),
        }));
    }

    let body_bytes = resp.body.unwrap_or_default();
    let body_json: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    let message_id = body_json
        .get("id")
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .unwrap_or_else(|| "bot-reply".to_string());
    let provider_message_id = format!("teams:{message_id}");

    json_bytes(&json!({
        "ok": true,
        "status": "replied",
        "provider_type": PROVIDER_TYPE,
        "public_base_url": cfg.public_base_url,
        "message_id": message_id,
        "provider_message_id": provider_message_id,
        "response": body_json,
    }))
}

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

    // Extract and validate JWT from Authorization header (Phase 1: decode-only)
    let auth_header = request
        .headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case("authorization"))
        .map(|h| h.value.as_str());

    // Load config to get Bot App ID for JWT validation
    let parsed_input: Value = serde_json::from_slice(input_json).unwrap_or(Value::Null);
    let cfg = load_config(&parsed_input).ok();

    if let (Some(auth), Some(config)) = (auth_header, &cfg) {
        if let Some(token) = extract_bearer_token(auth) {
            // Validate JWT (Phase 1: decode-only, no signature verification)
            if let Err(err) = validate_jwt(&token, &config.ms_bot_app_id) {
                // Log warning but don't fail - allow dev/testing without full validation
                eprintln!("JWT validation warning: {}", err);
            }
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
        // Bot Framework invoke/message with Action.Submit data
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
        return http_out_v1_bytes(&out);
    }

    // Bot Framework message activity (regular text messages)
    let text = extract_bot_text(&body_val);
    let team_id = extract_team_id(&body_val);
    let channel_id = extract_channel_id(&body_val);
    let user = extract_sender(&body_val);

    let mut envelope = build_team_envelope(text.clone(), user, team_id.clone(), channel_id.clone());

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

pub(crate) fn render_plan(input_json: &[u8]) -> Vec<u8> {
    render_plan_common(
        input_json,
        &RenderPlanConfig {
            capabilities: PlannerCapabilities {
                supports_adaptive_cards: true,
                supports_markdown: true,
                supports_html: true,
                supports_images: true,
                supports_buttons: true,
                max_text_len: None,
                max_payload_bytes: None,
            },
            default_summary: "teams message",
        },
    )
}

pub(crate) fn encode_op(input_json: &[u8]) -> Vec<u8> {
    let encode_message = match decode_encode_message(input_json) {
        Ok(value) => value,
        Err(err) => return encode_error(&err),
    };

    // Extract AC card from metadata if present
    let ac_json = encode_message.metadata.get("adaptive_card").cloned();

    // Serialize the full envelope so send_payload -> handle_send can parse it
    let mut envelope_val =
        serde_json::to_value(&encode_message).unwrap_or(Value::Object(Default::default()));

    if let Some(ac) = &ac_json {
        envelope_val
            .as_object_mut()
            .unwrap()
            .insert("_ac_json".to_string(), Value::String(ac.clone()));
    }

    // Forward reply_to_id from metadata
    if let Some(reply_id) = encode_message.metadata.get("reply_to_id") {
        let clean = reply_id.trim_matches('"');
        if !clean.is_empty() {
            envelope_val
                .as_object_mut()
                .unwrap()
                .insert("reply_to_id".to_string(), Value::String(clean.to_string()));
        }
    }

    // Forward serviceUrl and conversationId from metadata
    if let Some(service_url) = encode_message.metadata.get("serviceUrl") {
        envelope_val
            .as_object_mut()
            .unwrap()
            .insert("metadata".to_string(), json!({
                "serviceUrl": service_url,
                "conversationId": encode_message.metadata.get("conversationId").cloned().unwrap_or_default()
            }));
    }

    let body_bytes = serde_json::to_vec(&envelope_val).unwrap_or_else(|_| b"{}".to_vec());
    let mut metadata = BTreeMap::new();
    metadata.insert("method".to_string(), Value::String("POST".to_string()));

    let payload = ProviderPayloadV1 {
        content_type: "application/json".to_string(),
        body_b64: STANDARD.encode(&body_bytes),
        metadata,
    };
    json_bytes(&json!({"ok": true, "payload": payload}))
}

pub(crate) fn send_payload(input_json: &[u8]) -> Vec<u8> {
    let send_in = match serde_json::from_slice::<SendPayloadInV1>(input_json) {
        Ok(value) => value,
        Err(err) => {
            return send_payload_error(&format!("invalid send_payload input: {err}"), false);
        }
    };
    // Accept both messaging.teams.bot and messaging.teams.graph (manifest.cbor mismatch).
    if !send_in.provider_type.starts_with("messaging.teams") {
        return send_payload_error("provider type mismatch", false);
    }
    let payload_bytes = match STANDARD.decode(&send_in.payload.body_b64) {
        Ok(bytes) => bytes,
        Err(err) => {
            return send_payload_error(&format!("payload decode failed: {err}"), false);
        }
    };
    let payload: Value = serde_json::from_slice(&payload_bytes).unwrap_or(Value::Null);
    let payload_bytes = serde_json::to_vec(&payload).unwrap_or_else(|_| b"{}".to_vec());
    let result_bytes = handle_send(&payload_bytes);
    let result_value: Value = serde_json::from_slice(&result_bytes).unwrap_or(Value::Null);
    let ok = result_value
        .get("ok")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if ok {
        let msg_id = result_value
            .get("message_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        json_bytes(&json!({
            "ok": true,
            "message": msg_id,
            "retryable": false
        }))
    } else {
        let message = result_value
            .get("error")
            .and_then(Value::as_str)
            .map(|s| s.to_string())
            .unwrap_or_else(|| "send_payload failed".to_string());
        send_payload_error(&message, false)
    }
}

pub(crate) fn build_team_envelope(
    text: String,
    user_id: Option<String>,
    team_id: Option<String>,
    channel_id: Option<String>,
) -> ChannelMessageEnvelope {
    let env = EnvId::try_from("default").expect("env id");
    let tenant = TenantId::try_from("default").expect("tenant id");
    let mut metadata = MessageMetadata::new();
    metadata.insert("universal".to_string(), "true".to_string());
    if let Some(team) = &team_id {
        metadata.insert("team_id".to_string(), team.clone());
    }
    if let Some(channel) = &channel_id {
        metadata.insert("channel_id".to_string(), channel.clone());
    }
    if let Some(sender) = &user_id {
        metadata.insert("from".to_string(), sender.clone());
    }
    let channel_name = channel_id
        .clone()
        .or_else(|| team_id.clone())
        .unwrap_or_else(|| "teams".to_string());
    let sender = user_id.map(|id| Actor {
        id,
        kind: Some("user".into()),
    });
    ChannelMessageEnvelope {
        id: format!("teams-{channel_name}"),
        tenant: TenantCtx::new(env.clone(), tenant.clone()),
        channel: channel_name.clone(),
        session_id: channel_name,
        reply_scope: None,
        from: sender,
        to: {
            let mut dests = Vec::new();
            if let (Some(team), Some(channel)) = (&team_id, &channel_id) {
                dests.push(Destination {
                    id: format!("{}:{}", team, channel),
                    kind: Some("channel".to_string()),
                });
            }
            dests
        },
        correlation_id: None,
        text: Some(text),
        attachments: Vec::new(),
        metadata,
    }
}

pub(crate) fn build_team_envelope_from_input(
    parsed: &Value,
    cfg: &ProviderConfig,
) -> Result<ChannelMessageEnvelope, String> {
    let text = parsed
        .get("text")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| "text required".to_string())?;

    let destination = channel_destination(parsed, cfg)?;
    let team_id = parsed
        .get("team_id")
        .and_then(Value::as_str)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| cfg.team_id.clone());
    let channel_id = parsed
        .get("channel_id")
        .and_then(Value::as_str)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| cfg.channel_id.clone());
    let mut envelope = build_team_envelope(text, None, team_id.clone(), channel_id.clone());
    envelope.to = vec![destination];
    Ok(envelope)
}

pub(crate) fn channel_destination(
    parsed: &Value,
    cfg: &ProviderConfig,
) -> Result<Destination, String> {
    let kind = parsed
        .get("kind")
        .and_then(Value::as_str)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("channel");

    match kind {
        "channel" => {
            let team = parsed
                .get("team_id")
                .and_then(Value::as_str)
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .or_else(|| cfg.team_id.clone())
                .ok_or_else(|| "team_id required for channel destination".to_string())?;
            let channel = parsed
                .get("channel_id")
                .and_then(Value::as_str)
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .or_else(|| cfg.channel_id.clone())
                .ok_or_else(|| "channel_id required for channel destination".to_string())?;
            Ok(Destination {
                id: format!("{team}:{channel}"),
                kind: Some("channel".into()),
            })
        }
        "chat" => {
            let chat_id = parsed
                .get("chat_id")
                .and_then(Value::as_str)
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .ok_or_else(|| "chat_id required for chat destination".to_string())?;
            Ok(Destination {
                id: chat_id,
                kind: Some("chat".into()),
            })
        }
        "conversation" => {
            // Bot Framework conversation - use conversation_id directly
            let conversation_id = parsed
                .get("conversation_id")
                .or_else(|| parsed.get("conversationId"))
                .and_then(Value::as_str)
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .ok_or_else(|| {
                    "conversation_id required for conversation destination".to_string()
                })?;
            Ok(Destination {
                id: conversation_id,
                kind: Some("conversation".into()),
            })
        }
        other => Err(format!(
            "unsupported destination kind for envelope fallback: {other}"
        )),
    }
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
