use base64::{Engine, engine::general_purpose::STANDARD};
use greentic_types::messaging::universal_dto::SendPayloadInV1;
use provider_common::helpers::{
    PlannerCapabilities, RenderPlanConfig, decode_encode_message, encode_error, json_bytes,
    render_plan_common, send_payload_error, send_payload_success,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use urlencoding::encode as url_encode;

use crate::PROVIDER_TYPE;
use crate::auth;
use crate::config::{ProviderConfig, config_from_secrets, load_config};
use crate::graph::{graph_base_url, graph_post};
use greentic_types::{
    ChannelMessageEnvelope, Destination, EnvId, MessageMetadata, TenantCtx, TenantId,
};

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

    let envelope = match serde_json::from_slice::<ChannelMessageEnvelope>(input_json) {
        Ok(env) => {
            eprintln!("parsed envelope to={:?}", env.to);
            env
        }
        Err(err) => {
            eprintln!("fallback envelope due to parse error: {err}");
            build_channel_envelope(&parsed, &cfg)
        }
    };

    if !envelope.attachments.is_empty() {
        return json_bytes(&json!({"ok": false, "error": "attachments not supported"}));
    }

    let body = envelope
        .text
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let body = match body {
        Some(value) => value,
        None => return json_bytes(&json!({"ok": false, "error": "text required"})),
    };

    let destination = envelope.to.first().cloned().or_else(|| {
        cfg.default_to_address.clone().map(|addr| Destination {
            id: addr,
            kind: Some("email".into()),
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
    let kind = destination.kind.as_deref().unwrap_or("email");
    if kind != "email" && !kind.is_empty() {
        return json_bytes(&json!({
            "ok": false,
            "error": format!("unsupported destination kind: {kind}"),
        }));
    }

    let subject = envelope
        .metadata
        .get("subject")
        .cloned()
        .unwrap_or_else(|| "email message".to_string());

    let payload = json!({
        "from": cfg.from_address,
        "to": dest_id,
        "subject": subject,
        "body": body,
        "host": cfg.host,
        "port": cfg.port,
        "username": cfg.username,
        "tls_mode": cfg.tls_mode,
    });
    let hash = hex_sha256(&json_bytes(&payload));
    let message_id = pseudo_uuid_from_hex(&hash);
    let provider_message_id = format!("smtp:{hash}");

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

pub(crate) fn handle_reply(_input_json: &[u8]) -> Vec<u8> {
    let parsed: Value = match serde_json::from_slice(_input_json) {
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

    let to = match parsed.get("to").and_then(|v| v.as_str()) {
        Some(addr) if !addr.is_empty() => addr.to_string(),
        _ => return json_bytes(&json!({"ok": false, "error": "to required"})),
    };
    let subject = parsed
        .get("subject")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let body = parsed
        .get("body")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let thread_ref = parsed
        .get("reply_to_id")
        .or_else(|| parsed.get("thread_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let payload = json!({
        "from": cfg.from_address,
        "to": to,
        "subject": subject,
        "body": body,
        "in_reply_to": thread_ref,
        "host": cfg.host,
        "port": cfg.port,
        "username": cfg.username,
        "tls_mode": cfg.tls_mode,
    });
    let hash = hex_sha256(&json_bytes(&payload));
    let message_id = pseudo_uuid_from_hex(&hash);
    let provider_message_id = format!("smtp-reply:{hash}");

    json_bytes(&json!({
        "ok": true,
        "status": "replied",
        "provider_type": PROVIDER_TYPE,
        "public_base_url": cfg.public_base_url,
        "message_id": message_id,
        "provider_message_id": provider_message_id,
        "payload": payload
    }))
}

pub(crate) fn render_plan(input_json: &[u8]) -> Vec<u8> {
    render_plan_common(
        input_json,
        &RenderPlanConfig {
            capabilities: PlannerCapabilities {
                supports_adaptive_cards: false,
                supports_markdown: false,
                supports_html: true,
                supports_images: true,
                supports_buttons: false,
                max_text_len: None,
                max_payload_bytes: None,
            },
            default_summary: "email message",
        },
    )
}

pub(crate) fn encode_op(input_json: &[u8]) -> Vec<u8> {
    use provider_common::helpers::extract_ac_summary;

    let encode_message = match decode_encode_message(input_json) {
        Ok(value) => value,
        Err(err) => return encode_error(&err),
    };

    // If the message carries an Adaptive Card, convert to styled HTML email.
    let ac_html = encode_message
        .metadata
        .get("adaptive_card")
        .and_then(|ac_raw| ac_to_email_html(ac_raw));

    let (text, is_html) = if let Some(html) = ac_html {
        (html, true)
    } else {
        let fallback = encode_message
            .metadata
            .get("adaptive_card")
            .and_then(|ac_raw| {
                let caps = PlannerCapabilities {
                    supports_adaptive_cards: false,
                    supports_markdown: false,
                    supports_html: true,
                    supports_images: true,
                    supports_buttons: false,
                    max_text_len: None,
                    max_payload_bytes: None,
                };
                extract_ac_summary(ac_raw, &caps)
            })
            .or_else(|| encode_message.text.clone().filter(|t| !t.trim().is_empty()))
            .unwrap_or_else(|| "universal email payload".to_string());
        (fallback, false)
    };

    // Extract AC title for subject line if available.
    let ac_title = encode_message
        .metadata
        .get("adaptive_card")
        .and_then(|ac_raw| {
            let ac: Value = serde_json::from_str(ac_raw).ok()?;
            ac.get("body")
                .and_then(Value::as_array)?
                .iter()
                .find(|el| {
                    el.get("type").and_then(Value::as_str) == Some("TextBlock")
                        && (el
                            .get("weight")
                            .and_then(Value::as_str)
                            .is_some_and(|w| w.eq_ignore_ascii_case("bolder"))
                            || el
                                .get("style")
                                .and_then(Value::as_str)
                                .is_some_and(|s| s.eq_ignore_ascii_case("heading")))
                })
                .and_then(|el| el.get("text").and_then(Value::as_str))
                .map(|s| s.to_string())
        });

    // Extract destination email from envelope.to[0].id (preferred) or metadata
    let to = encode_message
        .to
        .first()
        .map(|d| d.id.clone())
        .or_else(|| encode_message.metadata.get("to").cloned())
        .unwrap_or_default();
    if to.is_empty() {
        return encode_error("missing email target");
    }
    let subject = encode_message
        .metadata
        .get("subject")
        .cloned()
        .or(ac_title)
        .unwrap_or_else(|| text.chars().take(78).collect::<String>());
    let mut payload_body = json!({
        "to": to.clone(),
        "subject": subject.clone(),
        "body": text,
    });
    if is_html {
        payload_body
            .as_object_mut()
            .unwrap()
            .insert("body_type".into(), json!("HTML"));
    }
    let body_bytes = serde_json::to_vec(&payload_body).unwrap_or_else(|_| b"{}".to_vec());
    let mut metadata = BTreeMap::new();
    metadata.insert("to".to_string(), Value::String(to));
    metadata.insert("subject".to_string(), Value::String(subject));
    metadata.insert("method".to_string(), Value::String("POST".to_string()));
    let metadata_json = serde_json::to_string(&metadata).unwrap_or_else(|_| "{}".to_string());
    json_bytes(&json!({
        "ok": true,
        "payload": {
            "content_type": "application/json",
            "body_b64": STANDARD.encode(&body_bytes),
            "metadata_json": metadata_json,
        }
    }))
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
    let payload_bytes: Vec<u8> = match STANDARD.decode(&send_in.payload.body_b64) {
        Ok(bytes) => bytes,
        Err(err) => {
            return send_payload_error(&format!("payload decode failed: {err}"), false);
        }
    };
    let payload: Value = serde_json::from_slice(&payload_bytes).unwrap_or(Value::Null);
    let subject = payload
        .get("subject")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let to = payload
        .get("to")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let body = payload
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if to.is_empty() {
        return send_payload_error("missing email target", false);
    }
    if subject.is_empty() {
        return send_payload_error("subject required", false);
    }
    // Build config from secrets store (reads all Graph credentials in one pass).
    let cfg = match config_from_secrets() {
        Ok(cfg) => cfg,
        Err(err) => return send_payload_error(&err, false),
    };
    let token = if let Some(user) = &send_in.auth_user {
        auth::acquire_graph_token(&cfg, user)
    } else {
        auth::acquire_graph_token_from_store(&cfg)
    };
    let token = match token {
        Ok(value) => value,
        Err(err) => return send_payload_error(&err, true),
    };
    let content_type = payload
        .get("body_type")
        .and_then(Value::as_str)
        .unwrap_or("Text");
    let mail_body = json!({
        "message": {
            "subject": subject,
            "body": { "contentType": content_type, "content": body },
            "toRecipients": [
                { "emailAddress": { "address": to } }
            ]
        },
        "saveToSentItems": false
    });
    // Use /me/sendMail for delegated tokens (refresh_token grant),
    // /users/{from}/sendMail for app-only tokens (client_credentials grant).
    let has_refresh_token = cfg
        .graph_refresh_token
        .as_ref()
        .is_some_and(|s| !s.is_empty());
    let url = if send_in.auth_user.is_some() || has_refresh_token {
        format!("{}/me/sendMail", graph_base_url(&cfg))
    } else {
        format!(
            "{}/users/{}/sendMail",
            graph_base_url(&cfg),
            url_encode(&cfg.from_address)
        )
    };
    if let Err(err) = graph_post(&token, &url, &mail_body) {
        return send_payload_error(&err, true);
    }
    send_payload_success()
}

fn build_channel_envelope(parsed: &Value, cfg: &ProviderConfig) -> ChannelMessageEnvelope {
    let to_addr = parsed
        .get("to")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            cfg.default_to_address
                .clone()
                .unwrap_or_else(|| "recipient@example.com".to_string())
        });
    let subject = parsed
        .get("subject")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "universal subject".to_string());
    let body_text = parsed
        .get("body")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let mut metadata = MessageMetadata::new();
    metadata.insert("to".to_string(), to_addr.clone());
    metadata.insert("subject".to_string(), subject.clone());
    ChannelMessageEnvelope {
        id: "synthetic-envelope".to_string(),
        tenant: TenantCtx::new(default_env(), default_tenant()),
        channel: PROVIDER_TYPE.to_string(),
        session_id: "synthetic-session".to_string(),
        reply_scope: None,
        from: None,
        to: vec![Destination {
            id: to_addr,
            kind: Some("email".to_string()),
        }],
        correlation_id: None,
        text: body_text,
        attachments: Vec::new(),
        metadata,
    }
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

fn default_env() -> EnvId {
    EnvId::try_from("default").expect("default env id present")
}

fn default_tenant() -> TenantId {
    TenantId::try_from("default").expect("default tenant id present")
}

// ─── Adaptive Card → Email HTML converter ───────────────────────────────

/// Convert an Adaptive Card JSON string into a styled HTML email body.
///
/// Email supports full HTML/CSS, so this produces the richest rendering:
/// - TextBlock → styled headings and paragraphs
/// - RichTextBlock → inline formatting (bold, italic, strikethrough, code, underline, links)
/// - Image/ImageSet → `<img>` tags
/// - FactSet → HTML table with bold keys
/// - ColumnSet → flexbox columns
/// - Container → `<div>` with border
/// - ActionSet + actions → styled link buttons
/// - Table → full HTML table
fn ac_to_email_html(ac_raw: &str) -> Option<String> {
    let ac: Value = serde_json::from_str(ac_raw).ok()?;
    let body = ac.get("body").and_then(Value::as_array);
    let top_actions = ac.get("actions").and_then(Value::as_array);

    let mut parts: Vec<String> = Vec::new();

    if let Some(body) = body {
        for element in body {
            email_element_to_html(element, &mut parts);
        }
    }
    if let Some(actions) = top_actions {
        let btns = email_action_buttons(actions);
        if !btns.is_empty() {
            parts.push(format!(
                "<div style=\"margin-top:16px;\">{}</div>",
                btns.join(" ")
            ));
        }
    }

    if parts.is_empty() {
        return None;
    }

    let inner = parts.join("\n");
    Some(format!(
        "<div style=\"font-family:Segoe UI,Helvetica,Arial,sans-serif;max-width:600px;\
         margin:0 auto;padding:20px;background:#fff;border:1px solid #e0e0e0;\
         border-radius:8px;\">\n{inner}\n</div>"
    ))
}

/// Convert a single AC body element to email HTML.
fn email_element_to_html(element: &Value, parts: &mut Vec<String>) {
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

            if is_heading || size == "extralarge" {
                parts.push(format!(
                    "<h1 style=\"margin:0 0 8px;color:#333;\">{escaped}</h1>"
                ));
            } else if is_bold || size == "large" {
                parts.push(format!(
                    "<h2 style=\"margin:0 0 8px;color:#333;\">{escaped}</h2>"
                ));
            } else if size == "medium" {
                parts.push(format!(
                    "<h3 style=\"margin:0 0 6px;color:#333;\">{escaped}</h3>"
                ));
            } else if is_subtle || size == "small" {
                parts.push(format!(
                    "<p style=\"margin:4px 0;color:#888;font-size:13px;\">{escaped}</p>"
                ));
            } else {
                parts.push(format!(
                    "<p style=\"margin:4px 0;color:#333;\">{escaped}</p>"
                ));
            }
        }

        "RichTextBlock" => {
            if let Some(inlines) = element.get("inlines").and_then(Value::as_array) {
                let mut html = String::new();
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
                        s = format!("<strong>{s}</strong>");
                    }
                    if inline
                        .get("italic")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    {
                        s = format!("<em>{s}</em>");
                    }
                    if inline
                        .get("strikethrough")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    {
                        s = format!("<del>{s}</del>");
                    }
                    if inline
                        .get("fontType")
                        .and_then(Value::as_str)
                        .is_some_and(|f| f.eq_ignore_ascii_case("monospace"))
                    {
                        s = format!(
                            "<code style=\"background:#f4f4f4;padding:2px 4px;\
                             border-radius:3px;\">{s}</code>"
                        );
                    }
                    if inline
                        .get("underline")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    {
                        s = format!("<u>{s}</u>");
                    }
                    if let Some(url) = inline.get("selectAction").and_then(|a| {
                        if a.get("type").and_then(Value::as_str) == Some("Action.OpenUrl") {
                            a.get("url").and_then(Value::as_str)
                        } else {
                            None
                        }
                    }) {
                        s = format!(
                            "<a href=\"{}\" style=\"color:#0078d4;\">{s}</a>",
                            html_escape(url)
                        );
                    }
                    html.push_str(&s);
                }
                if !html.is_empty() {
                    parts.push(format!("<p style=\"margin:4px 0;color:#333;\">{html}</p>"));
                }
            }
        }

        "Image" => {
            if let Some(url) = element.get("url").and_then(Value::as_str) {
                let alt = element
                    .get("altText")
                    .and_then(Value::as_str)
                    .unwrap_or("image");
                parts.push(format!(
                    "<div style=\"margin:8px 0;\"><img src=\"{}\" alt=\"{}\" \
                     style=\"max-width:100%;border-radius:4px;\" /></div>",
                    html_escape(url),
                    html_escape(alt)
                ));
            }
        }

        "ImageSet" => {
            if let Some(imgs) = element.get("images").and_then(Value::as_array) {
                let mut img_html = Vec::new();
                for img in imgs {
                    if let Some(url) = img.get("url").and_then(Value::as_str) {
                        let alt = img
                            .get("altText")
                            .and_then(Value::as_str)
                            .unwrap_or("image");
                        img_html.push(format!(
                            "<img src=\"{}\" alt=\"{}\" \
                             style=\"max-width:48%;border-radius:4px;margin:4px;\" />",
                            html_escape(url),
                            html_escape(alt)
                        ));
                    }
                }
                if !img_html.is_empty() {
                    parts.push(format!(
                        "<div style=\"margin:8px 0;\">{}</div>",
                        img_html.join("")
                    ));
                }
            }
        }

        "FactSet" => {
            if let Some(facts) = element.get("facts").and_then(Value::as_array) {
                let mut rows = Vec::new();
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
                        rows.push(format!(
                            "<tr><td style=\"padding:4px 12px 4px 0;font-weight:bold;\
                             color:#555;white-space:nowrap;\">{}</td>\
                             <td style=\"padding:4px 0;color:#333;\">{}</td></tr>",
                            html_escape(title),
                            html_escape(value)
                        ));
                    }
                }
                if !rows.is_empty() {
                    parts.push(format!(
                        "<table style=\"margin:8px 0;border-collapse:collapse;\">\
                         {}</table>",
                        rows.join("")
                    ));
                }
            }
        }

        "ColumnSet" => {
            if let Some(columns) = element.get("columns").and_then(Value::as_array) {
                let mut cols_html: Vec<String> = Vec::new();
                for col in columns {
                    if let Some(items) = col.get("items").and_then(Value::as_array) {
                        let mut col_parts: Vec<String> = Vec::new();
                        for item in items {
                            email_element_to_html(item, &mut col_parts);
                        }
                        if !col_parts.is_empty() {
                            cols_html.push(format!(
                                "<td style=\"vertical-align:top;padding:0 8px;\">{}</td>",
                                col_parts.join("")
                            ));
                        }
                    }
                }
                if !cols_html.is_empty() {
                    parts.push(format!(
                        "<table style=\"width:100%;margin:8px 0;\"><tr>{}</tr></table>",
                        cols_html.join("")
                    ));
                }
            }
        }

        "Container" => {
            if let Some(items) = element.get("items").and_then(Value::as_array) {
                let mut inner: Vec<String> = Vec::new();
                for item in items {
                    email_element_to_html(item, &mut inner);
                }
                if !inner.is_empty() {
                    parts.push(format!(
                        "<div style=\"margin:8px 0;padding:12px;border:1px solid #e8e8e8;\
                         border-radius:4px;background:#fafafa;\">{}</div>",
                        inner.join("")
                    ));
                }
            }
        }

        "ActionSet" => {
            if let Some(action_list) = element.get("actions").and_then(Value::as_array) {
                let btns = email_action_buttons(action_list);
                if !btns.is_empty() {
                    parts.push(format!(
                        "<div style=\"margin:8px 0;\">{}</div>",
                        btns.join(" ")
                    ));
                }
            }
        }

        "Table" => {
            let rows = element.get("rows").and_then(Value::as_array);
            let columns = element.get("columns").and_then(Value::as_array);
            if let Some(rows) = rows {
                let mut table_rows = Vec::new();
                // Header row
                if let Some(cols) = columns {
                    let headers: Vec<String> = cols
                        .iter()
                        .map(|c| {
                            let h = c
                                .get("title")
                                .or_else(|| c.get("header"))
                                .and_then(Value::as_str)
                                .unwrap_or("");
                            format!(
                                "<th style=\"padding:6px 12px;text-align:left;\
                                 border-bottom:2px solid #ddd;color:#555;\">{}</th>",
                                html_escape(h)
                            )
                        })
                        .collect();
                    if headers.iter().any(|h| !h.contains(">&lt;")) {
                        table_rows.push(format!("<tr>{}</tr>", headers.join("")));
                    }
                }
                for row in rows {
                    if let Some(cells) = row.get("cells").and_then(Value::as_array) {
                        let cell_html: Vec<String> = cells
                            .iter()
                            .map(|cell| {
                                let text = cell
                                    .get("items")
                                    .and_then(Value::as_array)
                                    .map(|items| {
                                        items
                                            .iter()
                                            .filter_map(|i| i.get("text").and_then(Value::as_str))
                                            .collect::<Vec<_>>()
                                            .join(" ")
                                    })
                                    .unwrap_or_default();
                                format!(
                                    "<td style=\"padding:6px 12px;\
                                     border-bottom:1px solid #eee;\">{}</td>",
                                    html_escape(&text)
                                )
                            })
                            .collect();
                        table_rows.push(format!("<tr>{}</tr>", cell_html.join("")));
                    }
                }
                if !table_rows.is_empty() {
                    parts.push(format!(
                        "<table style=\"width:100%;margin:8px 0;border-collapse:collapse;\">\
                         {}</table>",
                        table_rows.join("")
                    ));
                }
            }
        }

        _ => {}
    }
}

/// Convert AC actions to styled email button links.
fn email_action_buttons(action_list: &[Value]) -> Vec<String> {
    let mut btns = Vec::new();
    for action in action_list {
        let title = action
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if title.is_empty() {
            continue;
        }
        let atype = action
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let escaped = html_escape(title);
        if atype == "Action.OpenUrl" {
            let url = action.get("url").and_then(Value::as_str).unwrap_or("#");
            btns.push(format!(
                "<a href=\"{}\" style=\"display:inline-block;padding:8px 16px;\
                 background:#0078d4;color:#fff;text-decoration:none;\
                 border-radius:4px;margin:4px 4px 4px 0;font-size:14px;\">{escaped}</a>",
                html_escape(url)
            ));
        } else {
            // Non-URL actions rendered as disabled-style buttons.
            btns.push(format!(
                "<span style=\"display:inline-block;padding:8px 16px;\
                 background:#f0f0f0;color:#666;border-radius:4px;\
                 margin:4px 4px 4px 0;font-size:14px;\">{escaped}</span>"
            ));
        }
    }
    btns
}

/// Escape HTML special characters.
fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
