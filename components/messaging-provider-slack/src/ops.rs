use base64::{Engine as _, engine::general_purpose::STANDARD};
use greentic_types::messaging::universal_dto::{
    EncodeInV1, HttpInV1, HttpOutV1, ProviderPayloadV1, SendPayloadInV1,
};
use greentic_types::{
    Actor, ChannelMessageEnvelope, Destination, EnvId, MessageMetadata, TenantCtx, TenantId,
};
use provider_common::helpers::{
    encode_error, json_bytes, render_plan_common, send_payload_error, send_payload_success,
    RenderPlanConfig,
};
use provider_common::http_compat::{http_out_error, http_out_v1_bytes, parse_operator_http_in};
use serde_json::{Value, json};
use std::collections::BTreeMap;

use crate::bindings::greentic::http::http_client as client;
use crate::config::{
    ProviderConfig, get_secret_string, load_config, parse_destination, resolve_bot_token,
};
use crate::{DEFAULT_API_BASE, DEFAULT_BOT_TOKEN_KEY, PROVIDER_TYPE};

pub(crate) fn handle_send(input_json: &[u8], is_reply: bool) -> Vec<u8> {
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
            Err(message) => return json_bytes(&json!({"ok": false, "error": message})),
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
        cfg.default_channel.clone().map(|channel| Destination {
            id: channel,
            kind: Some("channel".into()),
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
    let kind = destination.kind.as_deref().unwrap_or("channel");
    if kind != "channel" && kind != "user" && !kind.is_empty() {
        return json_bytes(&json!({
            "ok": false,
            "error": format!("unsupported destination kind: {kind}")
        }));
    }

    let thread_ts = if is_reply {
        parsed
            .get("thread_id")
            .or_else(|| parsed.get("reply_to_id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    } else {
        None
    };

    let (format, blocks) = parse_blocks(&parsed);

    let token = resolve_bot_token(&cfg);
    let api_base = cfg
        .api_base_url
        .clone()
        .unwrap_or_else(|| DEFAULT_API_BASE.to_string());
    let url = format!("{}/chat.postMessage", api_base);
    let mut payload = json!({
        "channel": dest_id,
        "text": text,
    });
    if let Some(ts) = thread_ts {
        payload
            .as_object_mut()
            .expect("payload object")
            .insert("thread_ts".into(), Value::String(ts));
    }
    if format.as_deref() == Some("slack_blocks")
        && let Some(b) = blocks
    {
        payload
            .as_object_mut()
            .expect("payload object")
            .insert("blocks".into(), b);
    }

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
            &json!({"ok": false, "error": format!("slack returned status {}", resp.status)}),
        );
    }

    let body = resp.body.unwrap_or_default();
    let body_json: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let ts = body_json
        .get("ts")
        .or_else(|| body_json.get("message").and_then(|m| m.get("ts")))
        .and_then(|v| v.as_str())
        .unwrap_or("pending-ts")
        .to_string();
    let provider_message_id = format!("slack:{ts}");

    let result = json!({
        "ok": true,
        "status": if is_reply {"replied"} else {"sent"},
        "provider_type": PROVIDER_TYPE,
        "public_base_url": cfg.public_base_url,
        "message_id": ts,
        "provider_message_id": provider_message_id,
        "response": body_json
    });
    json_bytes(&result)
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
    // Slack Events API: {"type":"event_callback","event":{...}}
    // Legacy/generic:   {"body":{...}}
    // Flat:             {"text":"...","channel":"..."}
    let payload = body_val
        .get("event")
        .or_else(|| body_val.get("body"))
        .cloned()
        .unwrap_or_else(|| body_val.clone());
    let text = payload
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let channel = payload
        .get("channel")
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    let sender = payload
        .get("user")
        .or_else(|| payload.get("user_id"))
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    let envelope = build_slack_envelope(text, channel.clone(), sender);
    let normalized = json!({
        "ok": true,
        "event": body_val,
        "channel": channel,
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
            ac_tier: None,
            default_tier: "TierD",
            default_summary: "slack message",
            extract_ac_text: true,
        },
    )
}

pub(crate) fn encode_op(input_json: &[u8]) -> Vec<u8> {
    let encode_in = match serde_json::from_slice::<EncodeInV1>(input_json) {
        Ok(value) => value,
        Err(err) => return encode_error(&format!("invalid encode input: {err}")),
    };
    let channel = encode_in
        .message
        .to
        .first()
        .map(|d| d.id.clone())
        .unwrap_or_default();
    if channel.is_empty() {
        return encode_error("destination (to) required");
    }
    let text = encode_in
        .message
        .text
        .clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| "slack universal payload".to_string());
    let url = format!("{}/chat.postMessage", DEFAULT_API_BASE);
    let body = json!({
        "channel": channel,
        "text": text,
    });
    let body_bytes = serde_json::to_vec(&body).unwrap_or_else(|_| b"{}".to_vec());
    let mut metadata = BTreeMap::new();
    metadata.insert("url".to_string(), Value::String(url));
    metadata.insert("method".to_string(), Value::String("POST".to_string()));
    metadata.insert("channel".to_string(), Value::String(channel));
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
    let ProviderPayloadV1 {
        content_type,
        body_b64,
        metadata,
    } = send_in.payload;
    let url = metadata_string(&metadata, "url")
        .unwrap_or_else(|| format!("{}/chat.postMessage", DEFAULT_API_BASE));
    let method = metadata_string(&metadata, "method").unwrap_or_else(|| "POST".to_string());
    let body_bytes = match STANDARD.decode(&body_b64) {
        Ok(bytes) => bytes,
        Err(err) => return send_payload_error(&format!("payload decode failed: {err}"), false),
    };
    let token = match get_secret_string(DEFAULT_BOT_TOKEN_KEY) {
        Ok(value) => value,
        Err(err) => return send_payload_error(&err, false),
    };
    let request = client::Request {
        method,
        url,
        headers: vec![
            ("Content-Type".into(), content_type.clone()),
            ("Authorization".into(), format!("Bearer {token}")),
        ],
        body: Some(body_bytes),
    };
    let resp = match client::send(&request, None, None) {
        Ok(value) => value,
        Err(err) => {
            return send_payload_error(&format!("transport error: {}", err.message), true);
        }
    };
    if resp.status < 200 || resp.status >= 300 {
        return send_payload_error(
            &format!("slack returned status {}", resp.status),
            resp.status >= 500,
        );
    }
    send_payload_success()
}

pub(crate) fn parse_blocks(parsed: &Value) -> (Option<String>, Option<Value>) {
    let format = parsed
        .get("rich")
        .and_then(|v| v.get("format"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let blocks = parsed.get("rich").and_then(|v| v.get("blocks")).cloned();
    (format, blocks)
}

pub(crate) fn metadata_string(metadata: &BTreeMap<String, Value>, key: &str) -> Option<String> {
    metadata
        .get(key)
        .and_then(|value| value.as_str().map(|s| s.to_string()))
}

fn build_synthetic_envelope(
    parsed: &Value,
    cfg: &ProviderConfig,
) -> Result<ChannelMessageEnvelope, String> {
    let destination = parse_destination(parsed).or_else(|| {
        cfg.default_channel.clone().map(|channel| Destination {
            id: channel,
            kind: Some("channel".to_string()),
        })
    });
    let destination = destination.ok_or_else(|| "channel required".to_string())?;

    let env = EnvId::try_from("manual").expect("manual env id");
    let tenant = TenantId::try_from("manual").expect("manual tenant id");
    let mut metadata = MessageMetadata::new();
    metadata.insert("channel".to_string(), destination.id.clone());
    if let Some(kind) = &destination.kind {
        metadata.insert("destination_kind".to_string(), kind.clone());
    }

    let text = parsed
        .get("text")
        .and_then(|value| value.as_str())
        .map(|s| s.to_string());

    Ok(ChannelMessageEnvelope {
        id: "synthetic-slack-envelope".to_string(),
        tenant: TenantCtx::new(env, tenant),
        channel: destination.id.clone(),
        session_id: destination.id.clone(),
        reply_scope: None,
        from: None,
        to: vec![destination],
        correlation_id: None,
        text,
        attachments: Vec::new(),
        metadata,
    })
}

fn build_slack_envelope(
    text: String,
    channel: Option<String>,
    sender: Option<String>,
) -> ChannelMessageEnvelope {
    let env = EnvId::try_from("default").expect("env id");
    let tenant = TenantId::try_from("default").expect("tenant id");
    let mut metadata = MessageMetadata::new();
    metadata.insert("universal".to_string(), "true".to_string());
    if let Some(channel_id) = &channel {
        metadata.insert("channel".to_string(), channel_id.clone());
    }
    if let Some(sender_id) = &sender {
        metadata.insert("from".to_string(), sender_id.clone());
    }
    let channel_name = channel.clone().unwrap_or_else(|| "slack".to_string());
    let actor = sender.map(|id| Actor {
        id,
        kind: Some("user".into()),
    });
    let destinations = if let Some(ch) = &channel {
        vec![Destination {
            id: ch.clone(),
            kind: None,
        }]
    } else {
        Vec::new()
    };
    ChannelMessageEnvelope {
        id: format!("slack-{channel_name}"),
        tenant: TenantCtx::new(env.clone(), tenant.clone()),
        channel: channel_name.clone(),
        session_id: channel_name,
        reply_scope: None,
        from: actor,
        to: destinations,
        correlation_id: None,
        text: Some(text),
        attachments: Vec::new(),
        metadata,
    }
}
