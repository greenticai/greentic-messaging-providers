use base64::{Engine, engine::general_purpose::STANDARD};
use greentic_types::messaging::universal_dto::{
    EncodeInV1, HttpInV1, HttpOutV1, ProviderPayloadV1, SendPayloadInV1,
};
use greentic_types::{
    Actor, ChannelMessageEnvelope, Destination, EnvId, MessageMetadata, TenantCtx, TenantId,
};
use provider_common::helpers::{
    PlannerCapabilities, RenderPlanConfig, encode_error, json_bytes, render_plan_common,
    send_payload_error, send_payload_success,
};
use provider_common::http_compat::{http_out_error, http_out_v1_bytes, parse_operator_http_in};
use serde_json::{Value, json};
use std::collections::BTreeMap;

use crate::bindings::greentic::http::http_client as client;
use crate::config::{ProviderConfig, get_bot_token, load_config};
use crate::{DEFAULT_API_BASE, PROVIDER_TYPE};

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

    let envelope: ChannelMessageEnvelope = match serde_json::from_slice(input_json) {
        Ok(env) => env,
        Err(_) => match build_synthetic_envelope(&parsed, &cfg) {
            Ok(env) => env,
            Err(err) => return json_bytes(&json!({"ok": false, "error": err})),
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

    let destination = envelope.to.first().cloned().or_else(|| {
        cfg.default_chat_id.clone().map(|chat| Destination {
            id: chat,
            kind: Some("chat".into()),
        })
    });
    let destination = match destination {
        Some(dest) => dest,
        None => return json_bytes(&json!({"ok": false, "error": "destination required"})),
    };

    let dest_id = destination.id.trim();
    if dest_id.is_empty() {
        return json_bytes(&json!({"ok": false, "error": "destination id required"}));
    }
    let dest_id = dest_id.to_string();
    let kind = destination.kind.as_deref().unwrap_or("chat");
    if kind != "chat" && !kind.is_empty() {
        return json_bytes(&json!({
            "ok": false,
            "error": format!("unsupported destination kind: {kind}")
        }));
    }

    let token = match get_bot_token(&cfg) {
        Ok(s) => s,
        Err(err) => return json_bytes(&json!({"ok": false, "error": err})),
    };

    let api_base = cfg
        .api_base_url
        .clone()
        .unwrap_or_else(|| DEFAULT_API_BASE.to_string());

    // Build inline keyboard from AC actions (multiple buttons supported).
    let inline_keyboard = build_inline_keyboard_from_metadata(&envelope.metadata);
    let reply_markup = if !inline_keyboard.is_empty() {
        Some(json!({ "inline_keyboard": inline_keyboard }))
    } else {
        None
    };

    // Read parse_mode from metadata (set by encode_op for AC content).
    let parse_mode = envelope.metadata.get("parse_mode").cloned();

    // Check for AC images — use sendPhoto if available, otherwise sendMessage.
    let images: Vec<String> = envelope
        .metadata
        .get("ac_images")
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();

    // Build sendMessage payload (always needed as fallback).
    let text_payload = {
        let mut p = json!({
            "chat_id": dest_id.clone(),
            "text": text,
        });
        if let Some(pm) = &parse_mode {
            p.as_object_mut().unwrap().insert("parse_mode".into(), json!(pm));
        }
        if let Some(rm) = &reply_markup {
            p.as_object_mut().unwrap().insert("reply_markup".into(), rm.clone());
        }
        p
    };

    // Try sendPhoto if AC has images, fall back to sendMessage on failure.
    let resp = if let Some(photo_url) = images.first() {
        let url = format!("{api_base}/bot{token}/sendPhoto");
        let mut p = json!({
            "chat_id": dest_id.clone(),
            "photo": photo_url,
            "caption": text,
        });
        if let Some(pm) = &parse_mode {
            p.as_object_mut().unwrap().insert("parse_mode".into(), json!(pm));
        }
        if let Some(rm) = &reply_markup {
            p.as_object_mut().unwrap().insert("reply_markup".into(), rm.clone());
        }
        let req = client::Request {
            method: "POST".to_string(),
            url,
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: Some(serde_json::to_vec(&p).unwrap_or_else(|_| b"{}".to_vec())),
        };
        match client::send(&req, None, None) {
            Ok(r) if r.status >= 200 && r.status < 300 => r,
            _ => {
                // sendPhoto failed (bad image URL, etc.) — fall back to sendMessage.
                let url = format!("{api_base}/bot{token}/sendMessage");
                let req = client::Request {
                    method: "POST".to_string(),
                    url,
                    headers: vec![("Content-Type".into(), "application/json".into())],
                    body: Some(
                        serde_json::to_vec(&text_payload).unwrap_or_else(|_| b"{}".to_vec()),
                    ),
                };
                match client::send(&req, None, None) {
                    Ok(r) => r,
                    Err(err) => {
                        return json_bytes(&json!({
                            "ok": false,
                            "error": format!("transport error: {}", err.message),
                        }));
                    }
                }
            }
        }
    } else {
        let url = format!("{api_base}/bot{token}/sendMessage");
        let req = client::Request {
            method: "POST".to_string(),
            url,
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: Some(serde_json::to_vec(&text_payload).unwrap_or_else(|_| b"{}".to_vec())),
        };
        match client::send(&req, None, None) {
            Ok(r) => r,
            Err(err) => {
                return json_bytes(&json!({
                    "ok": false,
                    "error": format!("transport error: {}", err.message),
                }));
            }
        }
    };

    if resp.status < 200 || resp.status >= 300 {
        let body = resp.body.unwrap_or_default();
        let body_str = String::from_utf8_lossy(&body);
        return json_bytes(&json!({
            "ok": false,
            "error": format!("telegram returned status {}: {}", resp.status, body_str),
        }));
    }

    let body = resp.body.unwrap_or_default();
    let body_json: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let (message_id, provider_message_id) = extract_ids(&body_json);

    json_bytes(&json!({
        "ok": true,
        "status": "sent",
        "provider_type": PROVIDER_TYPE,
        "public_base_url": cfg.public_base_url,
        "message_id": message_id,
        "provider_message_id": provider_message_id,
        "response": body_json
    }))
}

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

    let text = match parsed
        .get("text")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
    {
        Some(t) if !t.is_empty() => t,
        _ => return json_bytes(&json!({"ok": false, "error": "text required"})),
    };

    let chat_id = match parsed
        .get("chat_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| cfg.default_chat_id.clone())
    {
        Some(chat) if !chat.is_empty() => chat,
        _ => return json_bytes(&json!({"ok": false, "error": "chat_id required"})),
    };

    let reply_to = parsed
        .get("reply_to_id")
        .or_else(|| parsed.get("thread_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if reply_to.is_empty() {
        return json_bytes(&json!({"ok": false, "error": "reply_to_id or thread_id required"}));
    }

    let token = match get_bot_token(&cfg) {
        Ok(s) => s,
        Err(err) => return json_bytes(&json!({"ok": false, "error": err})),
    };

    let api_base = cfg
        .api_base_url
        .clone()
        .unwrap_or_else(|| DEFAULT_API_BASE.to_string());
    let url = format!("{api_base}/bot{token}/sendMessage");
    let payload = json!({
        "chat_id": chat_id,
        "text": text,
        "reply_to_message_id": reply_to
    });
    let request = client::Request {
        method: "POST".to_string(),
        url,
        headers: vec![("Content-Type".to_string(), "application/json".to_string())],
        body: Some(serde_json::to_vec(&payload).unwrap_or_else(|_| b"{}".to_vec())),
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
            "error": format!("telegram returned status {}", resp.status),
        }));
    }

    let body_bytes = resp.body.unwrap_or_default();
    let body_json: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    let (message_id, provider_message_id) = extract_ids(&body_json);

    json_bytes(&json!({
        "ok": true,
        "status": "replied",
        "provider_type": PROVIDER_TYPE,
        "public_base_url": cfg.public_base_url,
        "message_id": message_id,
        "provider_message_id": provider_message_id,
        "response": body_json
    }))
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
    let body_bytes = match STANDARD.decode(&request.body_b64) {
        Ok(bytes) => bytes,
        Err(err) => return http_out_error(400, &format!("invalid body encoding: {err}")),
    };
    let body_val: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    let message = body_val.get("message").cloned().unwrap_or(Value::Null);
    let text = extract_message_text(&message);
    let chat_id = extract_chat_id(&message);
    let from = extract_from_user(&message);
    let envelope = build_telegram_envelope(text.clone(), chat_id.clone(), from.clone());
    let normalized = json!({
        "ok": true,
        "event": body_val,
        "message": message,
        "chat_id": chat_id,
        "from": from
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
    match std::panic::catch_unwind(|| render_plan_inner(input_json)) {
        Ok(result) => result,
        Err(err) => {
            eprintln!("telegram render_plan panic: {err:?}");
            std::panic::resume_unwind(err);
        }
    }
}

fn render_plan_inner(input_json: &[u8]) -> Vec<u8> {
    render_plan_common(
        input_json,
        &RenderPlanConfig {
            capabilities: PlannerCapabilities {
                supports_adaptive_cards: false,
                supports_markdown: true,
                supports_html: true,
                supports_images: true,
                supports_buttons: false,
                max_text_len: Some(4096),
                max_payload_bytes: None,
            },
            default_summary: "telegram message",
        },
    )
}

pub(crate) fn encode_op(input_json: &[u8]) -> Vec<u8> {
    use provider_common::helpers::extract_ac_plan;

    let encode_in = match serde_json::from_slice::<EncodeInV1>(input_json) {
        Ok(value) => value,
        Err(err) => return encode_error(&format!("invalid encode input: {err}")),
    };
    let mut envelope = encode_in.message;

    // If the message carries an Adaptive Card, build rich Telegram content:
    // - HTML-formatted text (bold title, escaped body)
    // - Inline keyboard buttons from AC actions
    // - Image URL for sendPhoto
    if let Some(ac_raw) = envelope.metadata.get("adaptive_card") {
        let caps = PlannerCapabilities {
            supports_adaptive_cards: false,
            supports_markdown: false,
            supports_html: true,
            supports_images: true,
            supports_buttons: true,
            max_text_len: Some(4096),
            max_payload_bytes: None,
        };
        if let Some(plan) = extract_ac_plan(ac_raw, &caps) {
            // Build HTML text: bold title + escaped body
            let html = build_html_text(plan.title.as_deref(), &plan.summary);
            envelope.text = Some(html);
            envelope
                .metadata
                .insert("parse_mode".to_string(), "HTML".to_string());
            if !plan.actions.is_empty() {
                let actions_json = serde_json::to_string(&plan.actions).unwrap_or_default();
                envelope
                    .metadata
                    .insert("ac_actions".to_string(), actions_json);
            }
            if !plan.images.is_empty() {
                let images_json = serde_json::to_string(&plan.images).unwrap_or_default();
                envelope
                    .metadata
                    .insert("ac_images".to_string(), images_json);
            }
        }
    }

    let has_text = envelope
        .text
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .is_some();
    if !has_text {
        envelope.text = Some("universal telegram payload".to_string());
    }
    let body_bytes = serde_json::to_vec(&envelope).unwrap_or_else(|_| b"{}".to_vec());
    let payload = ProviderPayloadV1 {
        content_type: "application/json".to_string(),
        body_b64: STANDARD.encode(&body_bytes),
        metadata: BTreeMap::new(),
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
    let payload_bytes = match STANDARD.decode(&send_in.payload.body_b64) {
        Ok(bytes) => bytes,
        Err(err) => {
            return send_payload_error(&format!("payload decode failed: {err}"), false);
        }
    };
    let payload: Value = serde_json::from_slice(&payload_bytes).unwrap_or(Value::Null);
    match forward_send_payload(&payload) {
        Ok(_) => send_payload_success(),
        Err(err) => send_payload_error(&err, false),
    }
}

pub(crate) fn forward_send_payload(payload: &Value) -> Result<(), String> {
    let payload_bytes =
        serde_json::to_vec(payload).map_err(|err| format!("serialize failed: {err}"))?;
    let result = handle_send(&payload_bytes);
    let result_value: Value =
        serde_json::from_slice(&result).map_err(|err| format!("parse send result: {err}"))?;
    let ok = result_value
        .get("ok")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if ok {
        Ok(())
    } else {
        let message = result_value
            .get("error")
            .and_then(Value::as_str)
            .map(|s| s.to_string())
            .unwrap_or_else(|| "send_payload failed".to_string());
        Err(message)
    }
}

pub(crate) fn build_telegram_envelope(
    text: String,
    chat_id: Option<String>,
    from: Option<String>,
) -> ChannelMessageEnvelope {
    let env = EnvId::try_from("default").expect("env id");
    let tenant = TenantId::try_from("default").expect("tenant id");
    let mut metadata = MessageMetadata::new();
    metadata.insert("universal".to_string(), "true".to_string());
    if let Some(chat) = &chat_id {
        metadata.insert("chat_id".to_string(), chat.clone());
    }
    if let Some(sender) = &from {
        metadata.insert("from".to_string(), sender.clone());
    }
    let channel = "telegram".to_string();
    let sender = from.map(|id| Actor {
        id,
        kind: Some("user".into()),
    });
    let destinations = if let Some(chat) = &chat_id {
        vec![Destination {
            id: chat.clone(),
            kind: Some("chat".into()),
        }]
    } else {
        Vec::new()
    };
    ChannelMessageEnvelope {
        id: format!("telegram-{channel}"),
        tenant: TenantCtx::new(env.clone(), tenant.clone()),
        channel: channel.clone(),
        session_id: chat_id.clone().unwrap_or_else(|| "telegram".to_string()),
        reply_scope: None,
        from: sender,
        to: destinations,
        correlation_id: None,
        text: Some(text),
        attachments: Vec::new(),
        metadata,
    }
}

fn build_synthetic_envelope(
    parsed: &Value,
    cfg: &ProviderConfig,
) -> Result<ChannelMessageEnvelope, String> {
    let text = parsed
        .get("text")
        .and_then(|value| value.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| "text required".to_string())?;

    let chat_id = parsed
        .get("chat_id")
        .and_then(|value| value.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| cfg.default_chat_id.clone())
        .ok_or_else(|| "chat_id required".to_string())?;

    let env = EnvId::try_from("synthetic").expect("manual env id");
    let tenant = TenantId::try_from("synthetic").expect("manual tenant id");
    let mut metadata = MessageMetadata::new();
    metadata.insert("chat_id".to_string(), chat_id.clone());
    metadata.insert("synthetic".to_string(), "true".to_string());

    let destination = Destination {
        id: chat_id.clone(),
        kind: Some("chat".to_string()),
    };

    Ok(ChannelMessageEnvelope {
        id: format!("synthetic-telegram-{chat_id}"),
        tenant: TenantCtx::new(env, tenant),
        channel: chat_id.clone(),
        session_id: chat_id.clone(),
        reply_scope: None,
        from: None,
        to: vec![destination],
        correlation_id: None,
        text: Some(text),
        attachments: Vec::new(),
        metadata,
    })
}

/// Build HTML-formatted text for Telegram from AC title + summary.
///
/// Title is rendered as `<b>title</b>`, body text is HTML-escaped.
/// Telegram supports: `<b>`, `<i>`, `<u>`, `<s>`, `<code>`, `<pre>`,
/// `<a href="url">text</a>`, `<blockquote>`.
fn build_html_text(title: Option<&str>, summary: &str) -> String {
    let mut parts = Vec::new();
    if let Some(t) = title {
        let t = t.trim();
        if !t.is_empty() {
            parts.push(format!("<b>{}</b>", html_escape(t)));
        }
    }
    // The summary may already contain the title as first line, skip it.
    let body = if let Some(t) = title {
        summary
            .strip_prefix(t.trim())
            .map(|rest| rest.trim_start_matches('\n'))
            .unwrap_or(summary)
    } else {
        summary
    };
    let body = body.trim();
    if !body.is_empty() {
        parts.push(html_escape(body));
    }
    parts.join("\n\n")
}

/// Escape HTML special characters for Telegram's HTML parse mode.
fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Build Telegram inline keyboard rows from AC actions stored in metadata.
///
/// Reads `metadata["ac_actions"]` (JSON array of `PlannerAction`), converts
/// `Action.OpenUrl` entries to inline URL buttons, other actions to callback
/// buttons. Supports multiple rows (max 8 rows, max 5 buttons per row).
fn build_inline_keyboard_from_metadata(
    metadata: &greentic_types::MessageMetadata,
) -> Vec<Vec<Value>> {
    let actions_json = match metadata.get("ac_actions") {
        Some(s) => s,
        None => return Vec::new(),
    };
    let actions: Vec<Value> = match serde_json::from_str(actions_json) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let max_rows = 8;
    let max_per_row = 3;
    let mut rows: Vec<Vec<Value>> = Vec::new();
    let mut current_row: Vec<Value> = Vec::new();
    for action in &actions {
        let title = action
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let url = action.get("url").and_then(Value::as_str);
        if title.is_empty() {
            continue;
        }
        if current_row.len() >= max_per_row {
            rows.push(current_row);
            current_row = Vec::new();
        }
        if rows.len() >= max_rows {
            break;
        }
        if let Some(url) = url {
            current_row.push(json!({"text": title, "url": url}));
        } else {
            // Callback button: callback_data max 64 bytes
            let cb: String = title.chars().take(64).collect();
            current_row.push(json!({"text": title, "callback_data": cb}));
        }
    }
    if !current_row.is_empty() && rows.len() < max_rows {
        rows.push(current_row);
    }
    rows
}

pub(crate) fn extract_message_text(value: &Value) -> String {
    value
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

pub(crate) fn extract_chat_id(value: &Value) -> Option<String> {
    value
        .get("chat")
        .and_then(|chat| chat.get("id"))
        .and_then(Value::as_i64)
        .map(|id| id.to_string())
}

pub(crate) fn extract_from_user(value: &Value) -> Option<String> {
    value
        .get("from")
        .and_then(|from| from.get("id"))
        .and_then(Value::as_i64)
        .map(|id| id.to_string())
}

pub(crate) fn extract_ids(body: &Value) -> (String, String) {
    let message_id = body
        .get("result")
        .and_then(|v| v.get("message_id"))
        .map(|val| match val {
            Value::Number(num) => num.to_string(),
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .unwrap_or_else(|| "dummy-message-id".into());
    let provider_message_id = format!("tg:{message_id}");
    (message_id, provider_message_id)
}
