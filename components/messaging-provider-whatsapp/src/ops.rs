use base64::{Engine as _, engine::general_purpose};
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
use std::collections::{BTreeMap, HashMap};

use crate::bindings::greentic::http::http_client as client;
use crate::config::{get_token, load_config};
use crate::{DEFAULT_API_BASE, DEFAULT_API_VERSION, PROVIDER_TYPE};

pub(crate) fn handle_send(input_json: &[u8]) -> Vec<u8> {
    let parsed: Value = match serde_json::from_slice(input_json) {
        Ok(val) => val,
        Err(err) => {
            return json_bytes(&json!({"ok": false, "error": format!("invalid json: {err}")}));
        }
    };

    if let Some(rich) = parsed.get("rich")
        && rich.get("format").and_then(Value::as_str) == Some("whatsapp_template")
    {
        return json_bytes(&json!({"ok": false, "error": "template messages not supported yet"}));
    }

    let cfg = match load_config(&parsed) {
        Ok(cfg) => cfg,
        Err(err) => return json_bytes(&json!({"ok": false, "error": err})),
    };
    if !cfg.enabled {
        return json_bytes(&json!({"ok": false, "error": "provider disabled by config"}));
    }

    let envelope: ChannelMessageEnvelope = match serde_json::from_slice(input_json) {
        Ok(env) => env,
        Err(err) => match build_send_envelope_from_input(&parsed) {
            Ok(env) => env,
            Err(message) => {
                return json_bytes(
                    &json!({"ok": false, "error": format!("invalid envelope: {message}: {err}")}),
                );
            }
        },
    };

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

    let destination = envelope.to.first().cloned();
    let destination = match destination {
        Some(dest) => dest,
        None => return json_bytes(&json!({"ok": false, "error": "destination required"})),
    };

    let dest_id = destination.id.trim();
    if dest_id.is_empty() {
        return json_bytes(&json!({"ok": false, "error": "destination id required"}));
    }
    let kind = destination.kind.as_deref().unwrap_or("phone");
    if kind != "phone" {
        return json_bytes(&json!({
            "ok": false,
            "error": format!("unsupported destination kind: {kind}"),
        }));
    }

    let token = match get_token(&cfg) {
        Ok(token) => token,
        Err(err) => return json_bytes(&json!({"ok": false, "error": err})),
    };

    let api_base = cfg
        .api_base_url
        .clone()
        .unwrap_or_else(|| DEFAULT_API_BASE.to_string());
    let api_version = cfg
        .api_version
        .clone()
        .unwrap_or_else(|| DEFAULT_API_VERSION.to_string());
    let url = format!(
        "{}/{}/{}/messages",
        api_base, api_version, cfg.phone_number_id
    );

    // Check for WhatsApp-specific rich content from AC conversion.
    let wa_buttons: Vec<Value> = parsed
        .get("wa_buttons")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let wa_image = parsed
        .get("wa_image")
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    let wa_header = parsed
        .get("wa_header")
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    let wa_video = parsed.get("wa_video").and_then(Value::as_str);
    let wa_video_caption = parsed.get("wa_video_caption").and_then(Value::as_str);
    let wa_audio = parsed.get("wa_audio").and_then(Value::as_str);
    let wa_document = parsed.get("wa_document").and_then(Value::as_str);
    let wa_document_filename = parsed.get("wa_document_filename").and_then(Value::as_str);
    let wa_document_caption = parsed.get("wa_document_caption").and_then(Value::as_str);
    let wa_sticker = parsed.get("wa_sticker").and_then(Value::as_str);
    let wa_location = parsed.get("wa_location");

    // Send media messages before the main text/interactive message.
    // Each media type is sent as a separate API call (WhatsApp Cloud API pattern).
    let mut media_results: Vec<Value> = Vec::new();

    if let Some(video_url) = wa_video {
        let mut video = json!({ "link": video_url });
        if let Some(cap) = wa_video_caption {
            let cap: String = cap.chars().take(1024).collect();
            video.as_object_mut().unwrap().insert("caption".into(), json!(cap));
        }
        let r = send_media_message(&url, &token, dest_id, &json!({
            "messaging_product": "whatsapp", "to": dest_id,
            "type": "video", "video": video
        }));
        media_results.push(json!({"type": "video", "ok": r.is_ok(), "detail": format!("{r:?}")}));
    }
    if let Some(audio_url) = wa_audio {
        let r = send_media_message(&url, &token, dest_id, &json!({
            "messaging_product": "whatsapp", "to": dest_id,
            "type": "audio", "audio": { "link": audio_url }
        }));
        media_results.push(json!({"type": "audio", "ok": r.is_ok(), "detail": format!("{r:?}")}));
    }
    if let Some(doc_url) = wa_document {
        let mut doc = json!({ "link": doc_url });
        if let Some(fname) = wa_document_filename {
            doc.as_object_mut().unwrap().insert("filename".into(), json!(fname));
        }
        if let Some(cap) = wa_document_caption {
            let cap: String = cap.chars().take(1024).collect();
            doc.as_object_mut().unwrap().insert("caption".into(), json!(cap));
        }
        let r = send_media_message(&url, &token, dest_id, &json!({
            "messaging_product": "whatsapp", "to": dest_id,
            "type": "document", "document": doc
        }));
        media_results.push(json!({"type": "document", "ok": r.is_ok(), "detail": format!("{r:?}")}));
    }
    if let Some(ref image_url) = wa_image {
        let caption: String = text.chars().take(1024).collect();
        let r = send_media_message(&url, &token, dest_id, &json!({
            "messaging_product": "whatsapp", "to": dest_id,
            "type": "image", "image": { "link": image_url, "caption": caption }
        }));
        media_results.push(json!({"type": "image", "ok": r.is_ok(), "detail": format!("{r:?}")}));
    }
    if let Some(sticker_url) = wa_sticker {
        let r = send_media_message(&url, &token, dest_id, &json!({
            "messaging_product": "whatsapp", "to": dest_id,
            "type": "sticker", "sticker": { "link": sticker_url }
        }));
        media_results.push(json!({"type": "sticker", "ok": r.is_ok(), "detail": format!("{r:?}")}));
    }
    if let Some(loc) = wa_location
        && loc.get("latitude").is_some()
        && loc.get("longitude").is_some()
    {
        let r = send_media_message(&url, &token, dest_id, &json!({
            "messaging_product": "whatsapp", "to": dest_id,
            "type": "location", "location": loc
        }));
        media_results.push(json!({"type": "location", "ok": r.is_ok(), "detail": format!("{r:?}")}));
    }

    // Build the main message payload.
    let payload = if !wa_buttons.is_empty() {
        // Interactive message with reply buttons (max 3).
        let buttons: Vec<Value> = wa_buttons
            .into_iter()
            .take(3)
            .enumerate()
            .map(|(i, btn)| {
                let title = btn.get("title").and_then(Value::as_str).unwrap_or("Button");
                let truncated: String = title.chars().take(20).collect();
                json!({
                    "type": "reply",
                    "reply": { "id": format!("btn_{i}"), "title": truncated }
                })
            })
            .collect();
        let body_text: String = text.chars().take(1024).collect();
        let mut interactive = json!({
            "type": "button",
            "body": { "text": body_text },
            "action": { "buttons": buttons }
        });
        if let Some(header) = wa_header {
            let h: String = header.chars().take(60).collect();
            interactive
                .as_object_mut()
                .unwrap()
                .insert("header".into(), json!({ "type": "text", "text": h }));
        }
        json!({
            "messaging_product": "whatsapp",
            "to": dest_id,
            "type": "interactive",
            "interactive": interactive
        })
    } else if wa_image.is_some() {
        // Image already sent above — skip text-only if no additional content.
        // But if there are facts/columns beyond the image caption, send text too.
        // We always send text as fallback after image.
        json!({
            "messaging_product": "whatsapp",
            "to": dest_id,
            "type": "text",
            "text": {"body": text},
        })
    } else {
        json!({
            "messaging_product": "whatsapp",
            "to": dest_id,
            "type": "text",
            "text": {"body": text},
        })
    };

    let request = client::Request {
        method: "POST".into(),
        url,
        headers: vec![
            ("Content-Type".into(), "application/json".into()),
            ("Authorization".into(), format!("Bearer {token}")),
        ],
        body: Some(serde_json::to_vec(&payload).unwrap_or_else(|_| b"{}".to_vec())),
    };

    let resp = match client::send(&request, None, None) {
        Ok(resp) => resp,
        Err(err) => {
            return json_bytes(
                &json!({"ok": false, "error": format!("transport error: {}", err.message)}),
            );
        }
    };

    if resp.status < 200 || resp.status >= 300 {
        return json_bytes(
            &json!({"ok": false, "error": format!("whatsapp returned status {}", resp.status)}),
        );
    }

    let body = resp.body.unwrap_or_default();
    let body_json: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let msg_id = body_json
        .get("messages")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|v| v.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("wa-message")
        .to_string();
    let provider_message_id = format!("whatsapp:{msg_id}");

    let mut result = json!({
        "ok": true,
        "status": "sent",
        "provider_type": PROVIDER_TYPE,
        "public_base_url": cfg.public_base_url,
        "message_id": msg_id,
        "provider_message_id": provider_message_id,
        "response": body_json
    });
    if !media_results.is_empty() {
        result.as_object_mut().unwrap().insert("media".into(), json!(media_results));
    }
    json_bytes(&result)
}

fn send_media_message(
    api_url: &str,
    token: &str,
    _dest_id: &str,
    payload: &Value,
) -> Result<Value, String> {
    let req = client::Request {
        method: "POST".into(),
        url: api_url.to_string(),
        headers: vec![
            ("Content-Type".into(), "application/json".into()),
            ("Authorization".into(), format!("Bearer {token}")),
        ],
        body: Some(serde_json::to_vec(payload).unwrap_or_default()),
    };
    match client::send(&req, None, None) {
        Ok(resp) => {
            let body = resp.body.unwrap_or_default();
            let body_json: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            if resp.status >= 200 && resp.status < 300 {
                Ok(body_json)
            } else {
                Err(format!("media send status {}: {}", resp.status, body_json))
            }
        }
        Err(err) => Err(format!("media transport error: {}", err.message)),
    }
}

fn build_send_envelope_from_input(parsed: &Value) -> Result<ChannelMessageEnvelope, String> {
    let text = parsed
        .get("text")
        .and_then(Value::as_str)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| "text required".to_string())?;
    let destination =
        parse_send_destination(parsed).ok_or_else(|| "destination required".to_string())?;
    let env = EnvId::try_from("manual").expect("manual env id");
    let tenant = TenantId::try_from("manual").expect("manual tenant id");
    let mut metadata = MessageMetadata::new();
    metadata.insert("synthetic".to_string(), "true".to_string());
    if let Some(kind) = destination.kind.as_ref() {
        metadata.insert("destination_kind".to_string(), kind.clone());
    }
    let channel = destination.id.clone();
    Ok(ChannelMessageEnvelope {
        id: format!("whatsapp-manual-{channel}"),
        tenant: TenantCtx::new(env, tenant),
        channel: channel.clone(),
        session_id: channel,
        reply_scope: None,
        from: None,
        to: vec![destination],
        correlation_id: None,
        text: Some(text),
        attachments: Vec::new(),
        metadata,
    })
}

fn parse_send_destination(parsed: &Value) -> Option<Destination> {
    let to_value = parsed.get("to")?;
    if let Some(id) = to_value.as_str() {
        let trimmed = id.trim();
        if trimmed.is_empty() {
            return None;
        }
        return Some(Destination {
            id: trimmed.to_string(),
            kind: Some("phone".to_string()),
        });
    }
    let obj = to_value.as_object()?;
    let id = obj
        .get("id")
        .and_then(|value| value.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let kind = obj
        .get("kind")
        .and_then(|value| value.as_str())
        .map(|s| s.trim().to_string());
    let kind = match kind.as_deref() {
        Some("user") => Some("phone".to_string()),
        Some(kind_str) if !kind_str.is_empty() => Some(kind_str.to_string()),
        _ => Some("phone".to_string()),
    };
    id.map(|id| Destination { id, kind })
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
    let to_kind = parsed
        .get("to")
        .and_then(|v| v.get("kind"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let to_id = parsed
        .get("to")
        .and_then(|v| v.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if to_kind != "user" || to_id.is_empty() {
        return json_bytes(&json!({"ok": false, "error": "to.kind=user with to.id required"}));
    }
    let text = parsed
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if text.is_empty() {
        return json_bytes(&json!({"ok": false, "error": "text required"}));
    }
    let reply_to = parsed
        .get("reply_to_id")
        .or_else(|| parsed.get("thread_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if reply_to.is_empty() {
        return json_bytes(&json!({"ok": false, "error": "reply_to_id or thread_id required"}));
    }

    let token = match get_token(&cfg) {
        Ok(token) => token,
        Err(err) => return json_bytes(&json!({"ok": false, "error": err})),
    };
    let api_base = cfg
        .api_base_url
        .clone()
        .unwrap_or_else(|| DEFAULT_API_BASE.to_string());
    let api_version = cfg
        .api_version
        .clone()
        .unwrap_or_else(|| DEFAULT_API_VERSION.to_string());
    let url = format!(
        "{}/{}/{}/messages",
        api_base, api_version, cfg.phone_number_id
    );
    let payload = json!({
        "messaging_product": "whatsapp",
        "to": to_id,
        "type": "text",
        "context": {"message_id": reply_to},
        "text": { "body": text }
    });
    let request = client::Request {
        method: "POST".into(),
        url,
        headers: vec![
            ("Content-Type".into(), "application/json".into()),
            ("Authorization".into(), format!("Bearer {token}")),
        ],
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
            "error": format!("whatsapp returned status {}", resp.status),
        }));
    }
    let body_bytes = resp.body.unwrap_or_default();
    let body_json: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    let msg_id = body_json
        .get("messages")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|v| v.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("wa-reply")
        .to_string();
    let provider_message_id = format!("whatsapp:{msg_id}");

    json_bytes(&json!({
        "ok": true,
        "status": "replied",
        "provider_type": PROVIDER_TYPE,
        "public_base_url": cfg.public_base_url,
        "message_id": msg_id,
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
    if request.method.eq_ignore_ascii_case("GET") {
        let challenge = parse_query(&request.query)
            .and_then(|params| params.get("hub.challenge").cloned())
            .unwrap_or_default();
        let out = HttpOutV1 {
            status: 200,
            headers: Vec::new(),
            body_b64: general_purpose::STANDARD.encode(challenge.as_bytes()),
            events: Vec::new(),
        };
        return http_out_v1_bytes(&out);
    }
    let body_bytes = match general_purpose::STANDARD.decode(&request.body_b64) {
        Ok(bytes) => bytes,
        Err(err) => return http_out_error(400, &format!("invalid body encoding: {err}")),
    };
    let body_val: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    // Extract message from Cloud API nested format: entry[].changes[].value.messages[]
    let cloud_msg = body_val
        .get("entry")
        .and_then(|e| e.as_array())
        .and_then(|arr| arr.first())
        .and_then(|e| e.get("changes"))
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|c| c.get("value"))
        .and_then(|v| v.get("messages"))
        .and_then(|m| m.as_array())
        .and_then(|arr| arr.first());
    // Use Cloud API message if available, otherwise fall back to flat format
    let msg = cloud_msg.unwrap_or(&body_val);
    let text = msg
        .get("text")
        .and_then(|t| t.get("body"))
        .and_then(Value::as_str)
        .or_else(|| msg.get("text").and_then(Value::as_str))
        .unwrap_or("")
        .to_string();
    let from = msg.get("from").and_then(Value::as_str).map(str::to_string);
    // Extract phone_number_id from Cloud API metadata
    let cloud_phone_id = body_val
        .get("entry")
        .and_then(|e| e.as_array())
        .and_then(|arr| arr.first())
        .and_then(|e| e.get("changes"))
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|c| c.get("value"))
        .and_then(|v| v.get("metadata"))
        .and_then(|m| m.get("phone_number_id"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let envelope = build_whatsapp_envelope(text.clone(), from.clone(), cloud_phone_id);
    let normalized = json!({
        "ok": true,
        "event": body_val,
        "text": text,
        "from": from,
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
                supports_adaptive_cards: false,
                supports_markdown: false,
                supports_html: false,
                supports_images: true,
                supports_buttons: false,
                max_text_len: Some(4096),
                max_payload_bytes: None,
            },
            default_summary: "whatsapp message",
        },
    )
}

pub(crate) fn encode_op(input_json: &[u8]) -> Vec<u8> {
    use provider_common::helpers::extract_ac_summary;

    let encode_in = match serde_json::from_slice::<EncodeInV1>(input_json) {
        Ok(value) => value,
        Err(err) => return encode_error(&format!("invalid encode input: {err}")),
    };

    // If the message carries an Adaptive Card, extract rich content for WhatsApp.
    let wa_content = encode_in
        .message
        .metadata
        .get("adaptive_card")
        .and_then(|ac_raw| ac_to_whatsapp(ac_raw));

    let text = if let Some(ref content) = wa_content {
        content.body.clone()
    } else {
        let caps = PlannerCapabilities {
            supports_adaptive_cards: false,
            supports_markdown: false,
            supports_html: false,
            supports_images: true,
            supports_buttons: false,
            max_text_len: Some(4096),
            max_payload_bytes: None,
        };
        encode_in
            .message
            .metadata
            .get("adaptive_card")
            .and_then(|ac_raw| extract_ac_summary(ac_raw, &caps))
            .or_else(|| {
                encode_in
                    .message
                    .text
                    .clone()
                    .filter(|t| !t.trim().is_empty())
            })
            .unwrap_or_else(|| "universal whatsapp payload".to_string())
    };

    // Destination: try metadata["from"] (ingress path), then message.to[0].id (demo send path)
    let to_id = encode_in
        .message
        .metadata
        .get("from")
        .cloned()
        .or_else(|| encode_in.message.to.first().map(|d| d.id.clone()))
        .unwrap_or_else(|| "whatsapp-user".to_string());
    let to_kind = encode_in
        .message
        .to
        .first()
        .and_then(|d| d.kind.clone())
        .unwrap_or_else(|| "phone".to_string());
    let phone_number_id = encode_in
        .message
        .metadata
        .get("phone_number_id")
        .cloned()
        .unwrap_or_else(|| "phone-universal".to_string());
    let to = json!({
        "kind": to_kind,
        "id": to_id,
    });
    let config = json!({
        "phone_number_id": phone_number_id,
        "enabled": true,
        "public_base_url": "https://localhost",
    });
    let mut payload_body = json!({
        "text": text,
        "to": to,
        "config": config,
    });
    // Store WhatsApp-specific content in the payload for handle_send.
    if let Some(content) = wa_content {
        let obj = payload_body.as_object_mut().unwrap();
        if let Some(header) = content.header {
            obj.insert("wa_header".into(), json!(header));
        }
        if let Some(image_url) = content.image_url {
            obj.insert("wa_image".into(), json!(image_url));
        }
        if !content.buttons.is_empty() {
            obj.insert("wa_buttons".into(), json!(content.buttons));
        }
    }

    // Enrich payload with wa_* media fields from message metadata.
    // Note: operator may wrap metadata values with extra quotes — strip them.
    {
        let meta = &encode_in.message.metadata;
        let obj = payload_body.as_object_mut().unwrap();
        let strip_quotes = |s: &str| -> String {
            s.strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .unwrap_or(s)
                .to_string()
        };

        if let Some(v) = meta.get("wa_video_url") {
            obj.insert("wa_video".into(), json!(strip_quotes(v)));
        }
        if let Some(v) = meta.get("wa_video_caption") {
            obj.insert("wa_video_caption".into(), json!(strip_quotes(v)));
        }
        if let Some(v) = meta.get("wa_audio_url") {
            obj.insert("wa_audio".into(), json!(strip_quotes(v)));
        }
        if let Some(v) = meta.get("wa_document_url") {
            obj.insert("wa_document".into(), json!(strip_quotes(v)));
        }
        if let Some(v) = meta.get("wa_document_filename") {
            obj.insert("wa_document_filename".into(), json!(strip_quotes(v)));
        }
        if let Some(v) = meta.get("wa_document_caption") {
            obj.insert("wa_document_caption".into(), json!(strip_quotes(v)));
        }
        if let Some(v) = meta.get("wa_sticker_url") {
            obj.insert("wa_sticker".into(), json!(strip_quotes(v)));
        }
        if let Some(v) = meta.get("wa_image_url") {
            obj.entry("wa_image").or_insert_with(|| json!(strip_quotes(v)));
        }

        // Location: build wa_location object from individual lat/lon/name/address metadata fields.
        if let (Some(lat), Some(lon)) = (
            meta.get("wa_location_latitude"),
            meta.get("wa_location_longitude"),
        ) {
            let mut loc = json!({ "latitude": strip_quotes(lat), "longitude": strip_quotes(lon) });
            if let Some(name) = meta.get("wa_location_name") {
                loc.as_object_mut().unwrap().insert("name".into(), json!(strip_quotes(name)));
            }
            if let Some(addr) = meta.get("wa_location_address") {
                loc.as_object_mut().unwrap().insert("address".into(), json!(strip_quotes(addr)));
            }
            obj.insert("wa_location".into(), loc);
        }

        // Map attachments by mime_type (only if corresponding wa_* not already set).
        for att in &encode_in.message.attachments {
            let mime = att.mime_type.as_str();
            if mime.starts_with("video/") && !obj.contains_key("wa_video") {
                obj.insert("wa_video".into(), json!(att.url));
                if let Some(ref name) = att.name {
                    obj.entry("wa_video_caption").or_insert_with(|| json!(name));
                }
            } else if mime.starts_with("audio/") && !obj.contains_key("wa_audio") {
                obj.insert("wa_audio".into(), json!(att.url));
            } else if mime == "image/webp" && !obj.contains_key("wa_sticker") {
                obj.insert("wa_sticker".into(), json!(att.url));
            } else if mime.starts_with("image/") && !obj.contains_key("wa_image") {
                obj.insert("wa_image".into(), json!(att.url));
            } else if !obj.contains_key("wa_document") {
                obj.insert("wa_document".into(), json!(att.url));
                if let Some(ref name) = att.name {
                    obj.insert("wa_document_filename".into(), json!(name));
                }
            }
        }
    }
    let body_bytes = serde_json::to_vec(&payload_body).unwrap_or_else(|_| b"{}".to_vec());
    let mut metadata = BTreeMap::new();
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

fn build_whatsapp_envelope(
    text: String,
    from: Option<String>,
    phone_number_id: Option<String>,
) -> ChannelMessageEnvelope {
    let env = EnvId::try_from("default").expect("env id");
    let tenant = TenantId::try_from("default").expect("tenant id");
    let mut metadata = MessageMetadata::new();
    metadata.insert("universal".to_string(), "true".to_string());
    metadata.insert("channel_id".to_string(), "whatsapp".to_string());
    let pnid = phone_number_id.unwrap_or_else(|| "unknown".to_string());
    metadata.insert("phone_number_id".to_string(), pnid);
    let sender = from.map(|id| Actor {
        id,
        kind: Some("user".into()),
    });
    if let Some(actor) = &sender {
        metadata.insert("from".to_string(), actor.id.clone());
    }
    let destinations = if let Some(actor) = &sender {
        vec![Destination {
            id: actor.id.clone(),
            kind: Some("phone".into()),
        }]
    } else {
        Vec::new()
    };
    ChannelMessageEnvelope {
        id: format!("whatsapp-{}", text),
        tenant: TenantCtx::new(env.clone(), tenant.clone()),
        channel: "whatsapp".to_string(),
        session_id: "whatsapp".to_string(),
        reply_scope: None,
        from: sender,
        to: destinations,
        correlation_id: None,
        text: Some(text),
        attachments: Vec::new(),
        metadata,
    }
}

// ─── Adaptive Card → WhatsApp converter ─────────────────────────────────

/// Extracted WhatsApp content from an Adaptive Card.
struct WhatsAppAcContent {
    header: Option<String>,
    body: String,
    buttons: Vec<Value>,
    image_url: Option<String>,
}

/// Convert an Adaptive Card JSON string into WhatsApp-native content.
///
/// WhatsApp supports:
/// - Interactive messages with reply buttons (max 3, title max 20 chars)
/// - Image messages with caption (max 1024 chars)
/// - Plain text (max 4096 chars)
fn ac_to_whatsapp(ac_raw: &str) -> Option<WhatsAppAcContent> {
    let ac: Value = serde_json::from_str(ac_raw).ok()?;
    let body_elements = ac.get("body").and_then(Value::as_array);
    let top_actions = ac.get("actions").and_then(Value::as_array);

    let mut header: Option<String> = None;
    let mut lines: Vec<String> = Vec::new();
    let mut buttons: Vec<Value> = Vec::new();
    let mut image_url: Option<String> = None;

    if let Some(elements) = body_elements {
        for element in elements {
            wa_extract_element(
                element,
                &mut header,
                &mut lines,
                &mut buttons,
                &mut image_url,
            );
        }
    }
    if let Some(actions) = top_actions {
        wa_collect_buttons(actions, &mut buttons);
    }

    let body = lines.join("\n");
    if body.trim().is_empty() {
        return None;
    }
    // WhatsApp body max 4096 for text, 1024 for interactive.
    let max = if buttons.is_empty() { 4096 } else { 1024 };
    let body: String = body.chars().take(max).collect();

    Some(WhatsAppAcContent {
        header,
        body,
        buttons,
        image_url,
    })
}

/// Extract content from a single AC element for WhatsApp.
fn wa_extract_element(
    element: &Value,
    header: &mut Option<String>,
    lines: &mut Vec<String>,
    buttons: &mut Vec<Value>,
    image_url: &mut Option<String>,
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

            if (is_bold || is_heading || size == "large" || size == "extralarge")
                && header.is_none()
            {
                *header = Some(text.to_string());
            } else {
                let formatted = if is_bold {
                    format!("*{text}*")
                } else {
                    text.to_string()
                };
                lines.push(formatted);
            }
        }

        "RichTextBlock" => {
            if let Some(inlines) = element.get("inlines").and_then(Value::as_array) {
                let mut line = String::new();
                for inline in inlines {
                    let text = inline
                        .get("text")
                        .and_then(Value::as_str)
                        .or_else(|| inline.as_str())
                        .unwrap_or_default();
                    if !text.is_empty() {
                        let mut s = text.to_string();
                        if inline
                            .get("fontWeight")
                            .and_then(Value::as_str)
                            .is_some_and(|w| w.eq_ignore_ascii_case("bolder"))
                        {
                            s = format!("*{s}*");
                        }
                        if inline
                            .get("italic")
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                        {
                            s = format!("_{s}_");
                        }
                        if inline
                            .get("strikethrough")
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                        {
                            s = format!("~{s}~");
                        }
                        if inline
                            .get("fontType")
                            .and_then(Value::as_str)
                            .is_some_and(|f| f.eq_ignore_ascii_case("monospace"))
                        {
                            s = format!("`{s}`");
                        }
                        line.push_str(&s);
                    }
                }
                if !line.is_empty() {
                    lines.push(line);
                }
            }
        }

        "Image" => {
            if image_url.is_none()
                && let Some(url) = element.get("url").and_then(Value::as_str)
            {
                *image_url = Some(url.to_string());
            }
        }

        "ImageSet" => {
            if image_url.is_none()
                && let Some(imgs) = element.get("images").and_then(Value::as_array)
                && let Some(url) = imgs
                    .first()
                    .and_then(|i| i.get("url"))
                    .and_then(Value::as_str)
            {
                *image_url = Some(url.to_string());
            }
        }

        "FactSet" => {
            if let Some(facts) = element.get("facts").and_then(Value::as_array) {
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
                        lines.push(format!("*{title}:* {value}"));
                    }
                }
            }
        }

        "ColumnSet" => {
            if let Some(columns) = element.get("columns").and_then(Value::as_array) {
                let mut col_texts: Vec<String> = Vec::new();
                for col in columns {
                    if let Some(items) = col.get("items").and_then(Value::as_array) {
                        let text: Vec<String> = items
                            .iter()
                            .filter_map(|i| {
                                i.get("text").and_then(Value::as_str).map(|s| s.to_string())
                            })
                            .collect();
                        if !text.is_empty() {
                            col_texts.push(text.join(" "));
                        }
                    }
                }
                if !col_texts.is_empty() {
                    lines.push(col_texts.join(" | "));
                }
            }
        }

        "Container" => {
            if let Some(items) = element.get("items").and_then(Value::as_array) {
                for item in items {
                    wa_extract_element(item, header, lines, buttons, image_url);
                }
            }
        }

        "ActionSet" => {
            if let Some(action_list) = element.get("actions").and_then(Value::as_array) {
                wa_collect_buttons(action_list, buttons);
            }
        }

        "Table" => {
            let rows = element.get("rows").and_then(Value::as_array);
            if let Some(rows) = rows {
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
                        lines.push(cell_texts.join(" | "));
                    }
                }
            }
        }

        _ => {}
    }
}

/// Collect AC actions into WhatsApp button format (max 3 reply buttons).
fn wa_collect_buttons(action_list: &[Value], buttons: &mut Vec<Value>) {
    for action in action_list {
        if buttons.len() >= 3 {
            break;
        }
        let title = action
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if title.is_empty() {
            continue;
        }
        buttons.push(json!({ "title": title }));
    }
}

pub(crate) fn parse_query(query: &Option<String>) -> Option<HashMap<String, String>> {
    let query = query.as_deref()?;
    let mut map = HashMap::new();
    for pair in query.split('&') {
        let mut parts = pair.splitn(2, '=');
        if let (Some(key), Some(value)) = (parts.next(), parts.next()) {
            map.insert(key.to_string(), value.to_string());
        }
    }
    if map.is_empty() { None } else { Some(map) }
}
