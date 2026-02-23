use base64::{Engine, engine::general_purpose::STANDARD};
use greentic_types::messaging::universal_dto::{
    EncodeInV1, HttpInV1, HttpOutV1, ProviderPayloadV1, SendPayloadInV1,
};
use greentic_types::{
    Actor, ChannelMessageEnvelope, Destination, EnvId, MessageMetadata, TenantCtx, TenantId,
};
use provider_common::helpers::{
    PlannerCapabilities, RenderPlanConfig, encode_error, json_bytes, render_plan_common,
    send_payload_error,
};
use provider_common::http_compat::{http_out_error, http_out_v1_bytes, parse_operator_http_in};
use serde_json::{Value, json};
use std::collections::BTreeMap;

use crate::bindings::greentic::http::http_client as client;
use crate::config::{ProviderConfig, default_channel_destination, load_config};
use crate::token::acquire_token;
use crate::{DEFAULT_GRAPH_BASE, PROVIDER_TYPE};

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

    let destination = envelope
        .to
        .first()
        .cloned()
        .or_else(|| default_channel_destination(&cfg));
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
    let graph_base = cfg
        .graph_base_url
        .clone()
        .unwrap_or_else(|| DEFAULT_GRAPH_BASE.to_string());

    // Reply-in-thread: check for reply_to_id in envelope or metadata.
    // Strip surrounding quotes — operator may double-quote args-json values.
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

    let url = match kind {
        "channel" => {
            let (team_id, channel_id) = match dest_id.split_once(':') {
                Some((team, channel)) => {
                    let team = team.trim();
                    let channel = channel.trim();
                    if team.is_empty() || channel.is_empty() {
                        return json_bytes(&json!({
                            "ok": false,
                            "error": "channel destination must include team_id and channel_id",
                        }));
                    }
                    (team.to_string(), channel.to_string())
                }
                None => {
                    return json_bytes(&json!({
                        "ok": false,
                        "error": "channel destination must be team_id:channel_id",
                    }));
                }
            };
            if let Some(ref msg_id) = reply_to_id {
                format!(
                    "{graph_base}/teams/{team_id}/channels/{channel_id}/messages/{msg_id}/replies"
                )
            } else {
                format!("{graph_base}/teams/{team_id}/channels/{channel_id}/messages")
            }
        }
        "chat" => {
            if dest_id.is_empty() {
                return json_bytes(&json!({"ok": false, "error": "destination id required"}));
            }
            format!("{graph_base}/chats/{dest_id}/messages")
        }
        other => {
            return json_bytes(&json!({
                "ok": false,
                "error": format!("unsupported destination kind: {other}"),
            }));
        }
    };

    let token = match acquire_token(&cfg) {
        Ok(tok) => tok,
        Err(err) => return json_bytes(&json!({"ok": false, "error": err})),
    };

    // Check if an Adaptive Card was injected by encode_op.
    let ac_json_str = parsed
        .get("_ac_json")
        .and_then(Value::as_str)
        .and_then(|s| serde_json::from_str::<Value>(s).ok());

    let body = if let Some(ac_card) = ac_json_str {
        // Send as native AC attachment — Teams renders it in full fidelity.
        json!({
            "body": {
                "content": "<attachment id=\"ac-card-1\"></attachment>",
                "contentType": "html"
            },
            "attachments": [{
                "id": "ac-card-1",
                "contentType": "application/vnd.microsoft.card.adaptive",
                "contentUrl": null,
                "content": serde_json::to_string(&ac_card).unwrap_or_default(),
                "name": null,
                "thumbnailUrl": null
            }]
        })
    } else {
        json!({
            "body": {
                "content": text,
                "contentType": "html"
            }
        })
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
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("");
        return json_bytes(&json!({
            "ok": false,
            "error": format!("graph returned status {}: {}", resp.status, err_msg),
            "response": err_body,
        }));
    }

    let body_bytes = resp.body.unwrap_or_default();
    let body_json: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    let message_id = body_json
        .get("id")
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .unwrap_or_else(|| "graph-message".to_string());
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
    let thread_id = parsed
        .get("reply_to_id")
        .or_else(|| parsed.get("thread_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if thread_id.is_empty() {
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

    let token = match acquire_token(&cfg) {
        Ok(tok) => tok,
        Err(err) => return json_bytes(&json!({"ok": false, "error": err})),
    };

    let graph_base = cfg
        .graph_base_url
        .clone()
        .unwrap_or_else(|| DEFAULT_GRAPH_BASE.to_string());
    let team_id = parsed
        .get("team_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| cfg.team_id.clone());
    let channel_id = parsed
        .get("channel_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| cfg.channel_id.clone());
    let (Some(team_id), Some(channel_id)) = (team_id, channel_id) else {
        return json_bytes(&json!({"ok": false, "error": "team_id and channel_id required"}));
    };

    let url = format!(
        "{}/teams/{}/channels/{}/messages/{}/replies",
        graph_base, team_id, channel_id, thread_id
    );
    let body = json!({
        "body": {
            "content": text,
            "contentType": "html"
        }
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
            "error": format!("graph returned status {}", resp.status),
        }));
    }
    let body_bytes = resp.body.unwrap_or_default();
    let body_json: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    let message_id = body_json
        .get("id")
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .unwrap_or_else(|| "graph-reply".to_string());
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
    let text = extract_team_text(&body_val);
    let team_id = extract_team_id(&body_val);
    let channel_id = extract_channel_id(&body_val);
    let user = extract_sender(&body_val);
    let envelope = build_team_envelope(text.clone(), user, team_id.clone(), channel_id.clone());
    let normalized = json!({
        "ok": true,
        "event": body_val,
        "team_id": team_id,
        "channel_id": channel_id,
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
    let encode_in = match serde_json::from_slice::<EncodeInV1>(input_json) {
        Ok(value) => value,
        Err(err) => return encode_error(&format!("invalid encode input: {err}")),
    };
    // Extract AC card from metadata if present — Teams renders it natively.
    let ac_json = encode_in.message.metadata.get("adaptive_card").cloned();

    // Serialize the full envelope so send_payload -> handle_send can parse it.
    // Inject ac_json into the serialized form so handle_send can attach it.
    let mut envelope_val =
        serde_json::to_value(&encode_in.message).unwrap_or(Value::Object(Default::default()));
    if let Some(ac) = &ac_json {
        envelope_val
            .as_object_mut()
            .unwrap()
            .insert("_ac_json".to_string(), Value::String(ac.clone()));
    }
    // Forward reply_to_id from metadata so handle_send can thread replies.
    // Strip surrounding quotes — operator may double-quote args-json values.
    if let Some(reply_id) = encode_in.message.metadata.get("reply_to_id") {
        let clean = reply_id.trim_matches('"');
        if !clean.is_empty() {
            envelope_val
                .as_object_mut()
                .unwrap()
                .insert("reply_to_id".to_string(), Value::String(clean.to_string()));
        }
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
    let payload_bytes = serde_json::to_vec(&payload).unwrap_or_else(|_| b"{}".to_vec());
    let result_bytes = handle_send(&payload_bytes);
    let result_value: Value = serde_json::from_slice(&result_bytes).unwrap_or(Value::Null);
    let ok = result_value
        .get("ok")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if ok {
        // Forward message_id so callers can use it for replies/threading.
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
        other => Err(format!(
            "unsupported destination kind for envelope fallback: {other}"
        )),
    }
}

pub(crate) fn extract_team_text(value: &Value) -> String {
    value
        .get("resourceData")
        .and_then(|rd| rd.get("body"))
        .and_then(|body| body.get("content"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

pub(crate) fn extract_team_id(value: &Value) -> Option<String> {
    value
        .get("resourceData")
        .and_then(|rd| rd.get("channelIdentity"))
        .and_then(|ci| ci.get("teamId"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

pub(crate) fn extract_channel_id(value: &Value) -> Option<String> {
    value
        .get("resourceData")
        .and_then(|rd| rd.get("channelIdentity"))
        .and_then(|ci| ci.get("channelId"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

pub(crate) fn extract_sender(value: &Value) -> Option<String> {
    value
        .get("resourceData")
        .and_then(|rd| rd.get("from"))
        .and_then(|from| from.get("user"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}
