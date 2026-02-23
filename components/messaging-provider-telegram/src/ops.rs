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
            p.as_object_mut()
                .unwrap()
                .insert("parse_mode".into(), json!(pm));
        }
        if let Some(rm) = &reply_markup {
            p.as_object_mut()
                .unwrap()
                .insert("reply_markup".into(), rm.clone());
        }
        p
    };

    // Choose API method based on content:
    //   1 image  → sendPhoto (photo + caption + buttons)
    //   2+ images → sendMediaGroup (album) + sendMessage (text + buttons)
    //   0 images → sendMessage (text + buttons)
    let resp = match images.len() {
        0 => {
            // No images: simple sendMessage.
            match tg_send_message(&api_base, &token, &text_payload) {
                Ok(r) => r,
                Err(err) => return json_bytes(&json!({"ok": false, "error": err})),
            }
        }
        1 => {
            // Single image: sendPhoto with caption (max 1024 chars) + buttons.
            let caption = truncate_html(&text, 1024);
            let mut p = json!({
                "chat_id": dest_id.clone(),
                "photo": images[0],
                "caption": caption,
            });
            if let Some(pm) = &parse_mode {
                p.as_object_mut()
                    .unwrap()
                    .insert("parse_mode".into(), json!(pm));
            }
            if let Some(rm) = &reply_markup {
                p.as_object_mut()
                    .unwrap()
                    .insert("reply_markup".into(), rm.clone());
            }
            let req = client::Request {
                method: "POST".to_string(),
                url: format!("{api_base}/bot{token}/sendPhoto"),
                headers: vec![("Content-Type".into(), "application/json".into())],
                body: Some(serde_json::to_vec(&p).unwrap_or_else(|_| b"{}".to_vec())),
            };
            match client::send(&req, None, None) {
                Ok(r) if (200..300).contains(&r.status) => r,
                _ => {
                    // sendPhoto failed — fall back to sendMessage.
                    match tg_send_message(&api_base, &token, &text_payload) {
                        Ok(r) => r,
                        Err(err) => return json_bytes(&json!({"ok": false, "error": err})),
                    }
                }
            }
        }
        _ => {
            // Multiple images: sendMediaGroup (album), then sendMessage (text + buttons).
            let media: Vec<Value> = images
                .iter()
                .take(10) // Telegram max 10 media per group
                .enumerate()
                .map(|(i, url)| {
                    if i == 0 {
                        // First item can have caption
                        let mut m = json!({"type": "photo", "media": url});
                        // Only short caption on album, full text in follow-up message
                        if let Some(pm) = &parse_mode {
                            m.as_object_mut()
                                .unwrap()
                                .insert("parse_mode".into(), json!(pm));
                        }
                        m
                    } else {
                        json!({"type": "photo", "media": url})
                    }
                })
                .collect();
            let album_payload = json!({
                "chat_id": dest_id.clone(),
                "media": media,
            });
            let album_req = client::Request {
                method: "POST".to_string(),
                url: format!("{api_base}/bot{token}/sendMediaGroup"),
                headers: vec![("Content-Type".into(), "application/json".into())],
                body: Some(serde_json::to_vec(&album_payload).unwrap_or_else(|_| b"{}".to_vec())),
            };
            // Send album (ignore errors — text message below is the important one).
            let _ = client::send(&album_req, None, None);
            // Follow up with text + inline keyboard.
            match tg_send_message(&api_base, &token, &text_payload) {
                Ok(r) => r,
                Err(err) => return json_bytes(&json!({"ok": false, "error": err})),
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
    let encode_in = match serde_json::from_slice::<EncodeInV1>(input_json) {
        Ok(value) => value,
        Err(err) => return encode_error(&format!("invalid encode input: {err}")),
    };
    let mut envelope = encode_in.message;

    // If the message carries an Adaptive Card, convert it to rich Telegram
    // content: HTML text, inline keyboard buttons, images for sendPhoto.
    if let Some(ac_raw) = envelope.metadata.get("adaptive_card")
        && let Some(content) = ac_to_telegram(ac_raw)
    {
        envelope.text = Some(content.html);
        envelope
            .metadata
            .insert("parse_mode".to_string(), "HTML".to_string());
        if !content.actions.is_empty() {
            let aj = serde_json::to_string(&content.actions).unwrap_or_default();
            envelope.metadata.insert("ac_actions".to_string(), aj);
        }
        if !content.images.is_empty() {
            let ij = serde_json::to_string(&content.images).unwrap_or_default();
            envelope.metadata.insert("ac_images".to_string(), ij);
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

// ─── Adaptive Card → Telegram HTML converter ───────────────────────────

/// Extracted Telegram content from an Adaptive Card.
struct TelegramAcContent {
    html: String,
    actions: Vec<Value>,
    images: Vec<String>,
}

/// Convert an Adaptive Card JSON string into rich Telegram HTML + actions + images.
///
/// Maps every AC element to its best Telegram-native representation:
/// - TextBlock → `<b>` for bold/heading, plain for normal, `<i>` for subtle
/// - RichTextBlock → inline formatting (`<b>`, `<i>`, `<s>`, `<code>`)
/// - Image/ImageSet → collected for sendPhoto/sendMediaGroup
/// - FactSet → `<b>key:</b> value` lines
/// - ColumnSet → columns separated by ` │ `
/// - Container → recursive processing
/// - ActionSet + top-level actions → inline keyboard buttons
/// - Table → `<pre>` formatted table
fn ac_to_telegram(ac_raw: &str) -> Option<TelegramAcContent> {
    let ac: Value = serde_json::from_str(ac_raw).ok()?;
    let body = ac.get("body").and_then(Value::as_array);
    let top_actions = ac.get("actions").and_then(Value::as_array);

    let mut html_parts: Vec<String> = Vec::new();
    let mut actions: Vec<Value> = Vec::new();
    let mut images: Vec<String> = Vec::new();

    if let Some(body) = body {
        for element in body {
            ac_element_to_html(element, &mut html_parts, &mut actions, &mut images);
        }
    }
    if let Some(top_actions) = top_actions {
        collect_actions(top_actions, &mut actions);
    }

    let html = html_parts.join("\n");
    if html.trim().is_empty() {
        return None;
    }

    // Telegram sendMessage max 4096 chars, sendPhoto caption max 1024 chars.
    // Truncate to 4096 for sendMessage; handle_send will further truncate for caption.
    let html = truncate_html(&html, 4096);

    Some(TelegramAcContent {
        html,
        actions,
        images,
    })
}

/// Recursively convert a single AC body element to Telegram HTML.
fn ac_element_to_html(
    element: &Value,
    parts: &mut Vec<String>,
    actions: &mut Vec<Value>,
    images: &mut Vec<String>,
) {
    let etype = element
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();

    match etype {
        "TextBlock" => {
            let text = element
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            if text.is_empty() {
                return;
            }
            let escaped = html_escape(text);
            let is_bold = element
                .get("weight")
                .and_then(Value::as_str)
                .is_some_and(|w| w.eq_ignore_ascii_case("bolder"));
            let size = element
                .get("size")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_ascii_lowercase();
            let is_heading = element
                .get("style")
                .and_then(Value::as_str)
                .is_some_and(|s| s.eq_ignore_ascii_case("heading"));
            let is_subtle = element
                .get("isSubtle")
                .and_then(Value::as_bool)
                .unwrap_or(false);

            let html = if is_bold || is_heading || size == "large" || size == "extralarge" {
                format!("<b>{escaped}</b>")
            } else if size == "small" || is_subtle {
                format!("<i>{escaped}</i>")
            } else {
                escaped
            };
            parts.push(html);
        }

        "RichTextBlock" => {
            let inlines = element.get("inlines").and_then(Value::as_array);
            if let Some(inlines) = inlines {
                let mut rich = String::new();
                for inline in inlines {
                    let text = inline
                        .get("text")
                        .and_then(Value::as_str)
                        .or_else(|| inline.as_str())
                        .unwrap_or_default();
                    if text.is_empty() {
                        continue;
                    }
                    let mut s = html_escape(text);
                    if inline
                        .get("fontWeight")
                        .and_then(Value::as_str)
                        .is_some_and(|w| w.eq_ignore_ascii_case("bolder"))
                    {
                        s = format!("<b>{s}</b>");
                    }
                    if inline
                        .get("italic")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    {
                        s = format!("<i>{s}</i>");
                    }
                    if inline
                        .get("strikethrough")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    {
                        s = format!("<s>{s}</s>");
                    }
                    if inline
                        .get("fontType")
                        .and_then(Value::as_str)
                        .is_some_and(|f| f.eq_ignore_ascii_case("monospace"))
                    {
                        s = format!("<code>{s}</code>");
                    }
                    if inline
                        .get("underline")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    {
                        s = format!("<u>{s}</u>");
                    }
                    // Check for hyperlink on TextRun
                    if let Some(url) = inline.get("selectAction").and_then(|a| {
                        if a.get("type").and_then(Value::as_str) == Some("Action.OpenUrl") {
                            a.get("url").and_then(Value::as_str)
                        } else {
                            None
                        }
                    }) {
                        s = format!("<a href=\"{}\">{s}</a>", html_escape(url));
                    }
                    rich.push_str(&s);
                }
                if !rich.is_empty() {
                    parts.push(rich);
                }
            }
        }

        "Image" => {
            if let Some(url) = element.get("url").and_then(Value::as_str) {
                images.push(url.to_string());
            }
        }

        "ImageSet" => {
            if let Some(imgs) = element.get("images").and_then(Value::as_array) {
                for img in imgs {
                    if let Some(url) = img.get("url").and_then(Value::as_str) {
                        images.push(url.to_string());
                    }
                }
            }
        }

        "FactSet" => {
            if let Some(facts) = element.get("facts").and_then(Value::as_array) {
                let mut lines = Vec::new();
                for fact in facts {
                    let title = fact
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let value = fact
                        .get("value")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if !title.is_empty() || !value.is_empty() {
                        lines.push(format!(
                            "<b>{}:</b> {}",
                            html_escape(title),
                            html_escape(value)
                        ));
                    }
                }
                if !lines.is_empty() {
                    parts.push(lines.join("\n"));
                }
            }
        }

        "ColumnSet" => {
            if let Some(columns) = element.get("columns").and_then(Value::as_array) {
                let mut col_texts: Vec<String> = Vec::new();
                for col in columns {
                    if let Some(items) = col.get("items").and_then(Value::as_array) {
                        let mut col_parts: Vec<String> = Vec::new();
                        for item in items {
                            ac_element_to_html(item, &mut col_parts, actions, images);
                        }
                        if !col_parts.is_empty() {
                            col_texts.push(col_parts.join("\n"));
                        }
                    }
                }
                if !col_texts.is_empty() {
                    parts.push(col_texts.join(" │ "));
                }
            }
        }

        "Container" => {
            if let Some(items) = element.get("items").and_then(Value::as_array) {
                for item in items {
                    ac_element_to_html(item, parts, actions, images);
                }
            }
        }

        "ActionSet" => {
            if let Some(action_list) = element.get("actions").and_then(Value::as_array) {
                collect_actions(action_list, actions);
            }
        }

        "Table" => {
            // Render table rows as pre-formatted text.
            let rows = element.get("rows").and_then(Value::as_array);
            let columns = element.get("columns").and_then(Value::as_array);
            if let Some(rows) = rows {
                let mut table_lines = Vec::new();
                // Header from column titles
                if let Some(cols) = columns {
                    let headers: Vec<String> = cols
                        .iter()
                        .map(|c| {
                            c.get("title")
                                .or_else(|| c.get("header"))
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string()
                        })
                        .collect();
                    if headers.iter().any(|h| !h.is_empty()) {
                        table_lines.push(headers.join(" │ "));
                        table_lines.push(
                            headers
                                .iter()
                                .map(|h| "─".repeat(h.len().max(3)))
                                .collect::<Vec<_>>()
                                .join("─┼─"),
                        );
                    }
                }
                for row in rows {
                    if let Some(cells) = row.get("cells").and_then(Value::as_array) {
                        let cell_texts: Vec<String> = cells
                            .iter()
                            .map(|cell| {
                                cell.get("items")
                                    .and_then(Value::as_array)
                                    .map(|items| {
                                        items
                                            .iter()
                                            .filter_map(|i| i.get("text").and_then(Value::as_str))
                                            .collect::<Vec<_>>()
                                            .join(" ")
                                    })
                                    .unwrap_or_default()
                            })
                            .collect();
                        table_lines.push(cell_texts.join(" │ "));
                    }
                }
                if !table_lines.is_empty() {
                    parts.push(format!(
                        "<pre>{}</pre>",
                        html_escape(&table_lines.join("\n"))
                    ));
                }
            }
        }

        _ => {
            // Unknown element type — ignore gracefully.
        }
    }
}

/// Collect AC actions into a flat JSON array for inline keyboard.
fn collect_actions(action_list: &[Value], actions: &mut Vec<Value>) {
    for action in action_list {
        let atype = action
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let title = action
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if title.is_empty() {
            continue;
        }
        match atype {
            "Action.OpenUrl" => {
                let url = action.get("url").and_then(Value::as_str).unwrap_or("");
                actions.push(json!({"title": title, "url": url}));
            }
            "Action.Submit" | "Action.Execute" => {
                actions.push(json!({"title": title}));
            }
            _ => {
                // Action.ShowCard, Action.ToggleVisibility — no Telegram equivalent.
                // Store as callback button so user at least sees the label.
                actions.push(json!({"title": title}));
            }
        }
    }
}

/// Escape HTML special characters for Telegram's HTML parse mode.
fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Truncate HTML string to at most `max` chars, preserving char boundaries.
fn truncate_html(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let truncated: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{truncated}\u{2026}")
}

/// Build Telegram inline keyboard rows from AC actions stored in metadata.
///
/// Supports multiple rows (max 8 rows, max 3 buttons per row).
/// URL buttons use `url` field, others use `callback_data` (max 64 bytes).
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
            let cb: String = title.chars().take(64).collect();
            current_row.push(json!({"text": title, "callback_data": cb}));
        }
    }
    if !current_row.is_empty() && rows.len() < max_rows {
        rows.push(current_row);
    }
    rows
}

/// Send a Telegram `sendMessage` request.
fn tg_send_message(
    api_base: &str,
    token: &str,
    payload: &Value,
) -> Result<client::Response, String> {
    let url = format!("{api_base}/bot{token}/sendMessage");
    let body = serde_json::to_vec(payload).unwrap_or_else(|_| b"{}".to_vec());
    let req = client::Request {
        method: "POST".to_string(),
        url,
        headers: vec![("Content-Type".into(), "application/json".into())],
        body: Some(body),
    };
    client::send(&req, None, None).map_err(|err| format!("transport error: {}", err.message))
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
