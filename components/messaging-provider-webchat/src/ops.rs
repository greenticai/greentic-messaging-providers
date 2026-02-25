use base64::{Engine as _, engine::general_purpose};
use greentic_types::messaging::universal_dto::{HttpInV1, HttpOutV1, ProviderPayloadV1, SendPayloadInV1};
use greentic_types::{Actor, ChannelMessageEnvelope, EnvId, MessageMetadata, TenantCtx, TenantId};
use provider_common::helpers::{
    PlannerCapabilities, RenderPlanConfig, decode_encode_message, encode_error, json_bytes, render_plan_common,
    send_payload_error, send_payload_success,
};
use provider_common::http_compat::{http_out_error, http_out_v1_bytes, parse_operator_http_in};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

use crate::PROVIDER_TYPE;
use crate::config::load_config;
use crate::directline::jwt::DirectLineContext;
use crate::directline::state::{StoredActivity, conversation_key};
use crate::directline::store::StateStore as _;
use crate::directline::{HostSecretStore, HostStateStore, handle_directline_request};

pub(crate) fn handle_send(input_json: &[u8]) -> Vec<u8> {
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

    let route = parsed
        .get("route")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| cfg.route.clone());
    let tenant_channel_id = parsed
        .get("tenant_channel_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| cfg.tenant_channel_id.clone());

    if route.is_none() && tenant_channel_id.is_none() {
        return json_bytes(&json!({"ok": false, "error": "route or tenant_channel_id required"}));
    }

    let text = parsed
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if text.is_empty() {
        return json_bytes(&json!({"ok": false, "error": "text required"}));
    }

    let payload = json!({
        "route": route,
        "tenant_channel_id": tenant_channel_id,
        "public_base_url": cfg.public_base_url,
        "mode": cfg.mode,
        "base_url": cfg.base_url,
        "text": text,
    });
    let payload_bytes = json_bytes(&payload);
    let key = route
        .clone()
        .or(tenant_channel_id.clone())
        .unwrap_or_else(|| "webchat".to_string());

    let mut state_store = HostStateStore;
    let write_result = state_store.write(&key, &payload_bytes);
    if let Err(err) = write_result {
        return json_bytes(&json!({"ok": false, "error": err}));
    }

    let hash_hex = hex_sha256(&payload_bytes);
    let message_id = pseudo_uuid_from_hex(&hash_hex);
    let provider_message_id = format!("webchat:{hash_hex}");

    json_bytes(&json!({
        "ok": true,
        "status": "sent",
        "provider_type": PROVIDER_TYPE,
        "public_base_url": cfg.public_base_url,
        "message_id": message_id,
        "provider_message_id": provider_message_id,
        "payload": payload
    }))
}

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
    // Extract Direct Line sub-path from operator-prefixed or direct paths.
    // Operator forwards full URI like /messaging/ingress/webchat/default/_/v3/directline/...
    if let Some(offset) = request.path.find("/v3/directline") {
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
        let secrets_driver = HostSecretStore;
        let mut out = handle_directline_request(&dl_request, &mut state_driver, &secrets_driver);

        // Emit ChannelMessageEnvelope for POST /activities so the operator can
        // forward user messages to the flow engine.
        if request.method.eq_ignore_ascii_case("POST")
            && dl_path.contains("/activities")
            && out.status == 201
            && let Ok(body_bytes) = general_purpose::STANDARD.decode(&request.body_b64)
            && let Ok(body) = serde_json::from_slice::<Value>(&body_bytes)
        {
            let text = extract_text(&body);
            let user = body
                .get("from")
                .and_then(|f| f.get("id"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            // Extract conversation_id from the path
            let conv_id = dl_path
                .strip_prefix("/v3/directline/conversations/")
                .and_then(|rest| rest.split('/').next())
                .map(|s| s.to_string());
            if !text.is_empty() {
                let envelope = build_webchat_envelope(text, user, conv_id.clone(), None);
                out.events.push(envelope);
            }
        }

        return http_out_v1_bytes(&out);
    }
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
            default_summary: "webchat message",
        },
    )
}

pub(crate) fn encode_op(input_json: &[u8]) -> Vec<u8> {
    let encode_message = match decode_encode_message(input_json) {
        Ok(value) => value,
        Err(err) => return encode_error(&err),
    };
    let text = encode_message
        .text
        .clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| "webchat universal payload".to_string());
    let metadata_route = encode_message.metadata.get("route").cloned();
    let route = metadata_route
        .clone()
        .or_else(|| Some(encode_message.session_id.clone()));
    let route_value = route.clone().unwrap_or_else(|| "webchat".to_string());
    let payload_body = json!({
        "text": text,
        "route": route_value.clone(),
        "session_id": encode_message.session_id,
    });
    let body_bytes = serde_json::to_vec(&payload_body).unwrap_or_else(|_| b"{}".to_vec());
    let mut metadata = BTreeMap::new();
    metadata.insert("route".to_string(), Value::String(route_value.clone()));
    metadata.insert("method".to_string(), Value::String("POST".to_string()));
    let payload = ProviderPayloadV1 {
        content_type: "application/json".to_string(),
        body_b64: general_purpose::STANDARD.encode(&body_bytes),
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
    if send_in.provider_type != PROVIDER_TYPE {
        return send_payload_error("provider type mismatch", false);
    }
    let payload_bytes = match general_purpose::STANDARD.decode(&send_in.payload.body_b64) {
        Ok(bytes) => bytes,
        Err(err) => {
            return send_payload_error(&format!("payload decode failed: {err}"), false);
        }
    };
    let payload: Value = serde_json::from_slice(&payload_bytes).unwrap_or(Value::Null);
    match persist_send_payload(&payload) {
        Ok(_) => send_payload_success(),
        Err(err) => send_payload_error(&err, false),
    }
}

fn persist_send_payload(payload: &Value) -> Result<(), String> {
    let route = route_from_value(payload);
    let tenant_channel_id = tenant_channel_from_value(payload);
    let key = route
        .clone()
        .or(tenant_channel_id.clone())
        .ok_or_else(|| "route or tenant_channel_id required".to_string())?;
    let text = extract_text(payload);
    if text.is_empty() {
        return Err("text required".into());
    }

    // If session_id is present, try to append the bot response as a Direct Line
    // activity so that GET /activities polling returns it to the frontend.
    if let Some(session_id) = value_as_trimmed_string(payload.get("session_id")) {
        let _ = append_bot_activity_to_conversation(&session_id, &text);
    }

    let public_base_url = public_base_url_from_value(payload);
    let stored = json!({
        "route": route,
        "tenant_channel_id": tenant_channel_id,
        "public_base_url": public_base_url,
        "mode": value_as_trimmed_string(payload.get("mode")).unwrap_or_else(|| "local_queue".to_string()),
        "base_url": value_as_trimmed_string(payload.get("base_url")),
        "text": text,
    });
    let mut state_store = HostStateStore;
    state_store.write(&key, &json_bytes(&stored))?;
    Ok(())
}

/// Append a bot-originated activity to the Direct Line conversation state.
/// Uses default context (env=default, tenant=default, team=_) matching the demo setup.
/// Best-effort: silently ignores errors (conversation may not exist).
fn append_bot_activity_to_conversation(conversation_id: &str, text: &str) -> Result<(), String> {
    let ctx = DirectLineContext {
        env: "default".into(),
        tenant: "default".into(),
        team: None,
    };
    let conv_key = conversation_key(&ctx, conversation_id);
    let mut store = HostStateStore;

    let conv_bytes = match store.read(&conv_key) {
        Ok(Some(bytes)) => bytes,
        _ => return Ok(()), // conversation not found, skip silently
    };

    let mut conversation: crate::directline::state::ConversationState =
        serde_json::from_slice(&conv_bytes).map_err(|e| e.to_string())?;

    let watermark = conversation.bump_watermark();
    let activity = StoredActivity {
        id: format!("bot-{watermark}"),
        type_: "message".to_string(),
        text: Some(text.to_string()),
        from: Some("bot".to_string()),
        timestamp: chrono::Utc::now().timestamp_millis(),
        watermark,
        raw: json!({
            "type": "message",
            "text": text,
            "from": {"id": "bot", "name": "Bot"},
        }),
    };
    conversation.activities.push(activity);

    let updated = serde_json::to_vec(&conversation).map_err(|e| e.to_string())?;
    store.write(&conv_key, &updated)?;
    Ok(())
}

fn build_webchat_envelope(
    text: String,
    user_id: Option<String>,
    route: Option<String>,
    tenant_channel_id: Option<String>,
) -> ChannelMessageEnvelope {
    let env = EnvId::try_from("default").expect("env id");
    let tenant = TenantId::try_from("default").expect("tenant id");
    let mut metadata = MessageMetadata::new();
    metadata.insert("universal".to_string(), "true".to_string());
    if let Some(route) = &route {
        metadata.insert("route".to_string(), route.clone());
    }
    if let Some(channel) = &tenant_channel_id {
        metadata.insert("tenant_channel_id".to_string(), channel.clone());
    }
    let channel = route
        .clone()
        .or_else(|| tenant_channel_id.clone())
        .unwrap_or_else(|| "webchat".to_string());
    ChannelMessageEnvelope {
        id: format!("webchat-{channel}"),
        tenant: TenantCtx::new(env.clone(), tenant.clone()),
        channel: channel.clone(),
        session_id: channel,
        reply_scope: None,
        from: user_id.map(|id| Actor {
            id,
            kind: Some("user".into()),
        }),
        to: Vec::new(),
        correlation_id: None,
        text: Some(text),
        attachments: Vec::new(),
        metadata,
    }
}

fn extract_text(value: &Value) -> String {
    value
        .get("text")
        .or_else(|| value.get("message"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn user_from_value(value: &Value) -> Option<String> {
    value
        .get("user_id")
        .or_else(|| value.get("from"))
        .and_then(|v| v.as_str())
        .and_then(|s| non_empty_string(Some(s)))
}

fn route_from_value(value: &Value) -> Option<String> {
    value_as_trimmed_string(value.get("route"))
}

fn tenant_channel_from_value(value: &Value) -> Option<String> {
    value_as_trimmed_string(value.get("tenant_channel_id"))
}

fn public_base_url_from_value(value: &Value) -> Option<String> {
    value_as_trimmed_string(value.get("public_base_url"))
}

fn value_as_trimmed_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

fn non_empty_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

pub(crate) fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(&mut out, "{:02x}", byte);
    }
    out
}

pub(crate) fn pseudo_uuid_from_hex(hex: &str) -> String {
    let padded = if hex.len() < 32 {
        format!("{hex:0<32}")
    } else {
        hex[..32].to_string()
    };
    format!(
        "{}-{}-{}-{}-{}",
        &padded[0..8],
        &padded[8..12],
        &padded[12..16],
        &padded[16..20],
        &padded[20..32]
    )
}
