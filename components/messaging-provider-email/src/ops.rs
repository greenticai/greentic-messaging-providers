use base64::{Engine, engine::general_purpose::STANDARD};
use greentic_types::messaging::universal_dto::{
    EncodeInV1, ProviderPayloadV1, SendPayloadInV1,
};
use provider_common::helpers::{
    encode_error, json_bytes, render_plan_common, send_payload_error,
    send_payload_success, RenderPlanConfig,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use urlencoding::encode as url_encode;

use crate::auth;
use crate::config::{ProviderConfig, config_from_secrets, load_config, parse_config_value};
use crate::graph::{graph_base_url, graph_post};
use crate::PROVIDER_TYPE;
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
            ac_tier: None,
            default_tier: "TierD",
            default_summary: "email message",
            extract_ac_text: true,
        },
    )
}

pub(crate) fn encode_op(input_json: &[u8]) -> Vec<u8> {
    let encode_in = match serde_json::from_slice::<EncodeInV1>(input_json) {
        Ok(value) => value,
        Err(err) => return encode_error(&format!("invalid encode input: {err}")),
    };
    let text = encode_in
        .message
        .text
        .clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| "universal email payload".to_string());
    // Extract destination email from envelope.to[0].id (preferred) or metadata
    let to = encode_in
        .message
        .to
        .first()
        .map(|d| d.id.clone())
        .or_else(|| encode_in.message.metadata.get("to").cloned())
        .unwrap_or_default();
    if to.is_empty() {
        return encode_error("missing email target");
    }
    let subject = encode_in
        .message
        .metadata
        .get("subject")
        .cloned()
        .unwrap_or_else(|| {
            // Use text as subject if no explicit subject
            text.chars().take(78).collect::<String>()
        });
    let payload_body = json!({
        "to": to.clone(),
        "subject": subject.clone(),
        "body": text,
    });
    let body_bytes = serde_json::to_vec(&payload_body).unwrap_or_else(|_| b"{}".to_vec());
    let mut metadata = BTreeMap::new();
    metadata.insert("to".to_string(), Value::String(to));
    metadata.insert("subject".to_string(), Value::String(subject));
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
    let mut config_value = serde_json::Map::new();
    for key in [
        "enabled",
        "public_base_url",
        "host",
        "port",
        "username",
        "from_address",
        "tls_mode",
        "password",
        "graph_tenant_id",
        "graph_authority",
        "graph_base_url",
        "graph_token_endpoint",
        "graph_scope",
    ] {
        if let Some(value) = send_in.payload.metadata.get(key) {
            config_value.insert(key.to_string(), value.clone());
        }
    }
    let cfg = if !config_value.is_empty() {
        match parse_config_value(&Value::Object(config_value)) {
            Ok(cfg) => cfg,
            Err(err) => return send_payload_error(&err, false),
        }
    } else {
        // Metadata unavailable (operator uses metadata_json, not metadata).
        // Build a minimal config from secrets for Graph API send.
        match config_from_secrets() {
            Ok(cfg) => cfg,
            Err(err) => return send_payload_error(&err, false),
        }
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
    let mail_body = json!({
        "message": {
            "subject": subject,
            "body": { "contentType": "Text", "content": body },
            "toRecipients": [
                { "emailAddress": { "address": to } }
            ]
        },
        "saveToSentItems": false
    });
    // Use /me/sendMail for delegated tokens, /users/{from}/sendMail for app-only
    let url = if send_in.auth_user.is_some() {
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
