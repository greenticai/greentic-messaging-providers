use base64::{Engine as _, engine::general_purpose::STANDARD};
use greentic_types::messaging::universal_dto::{
    HttpInV1, HttpOutV1, ProviderPayloadV1, SendPayloadInV1,
};
use greentic_types::{
    Actor, ChannelMessageEnvelope, Destination, EnvId, MessageMetadata, TenantCtx, TenantId,
};
use provider_common::helpers::{
    PlannerCapabilities, RenderPlanConfig, decode_encode_message, encode_error, json_bytes,
    render_plan_common, send_payload_error, send_payload_success,
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

    // Slack returns HTTP 200 even on errors — check the JSON body.
    if body_json.get("ok").and_then(Value::as_bool) == Some(false) {
        let err = body_json
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("unknown slack error");
        return json_bytes(
            &json!({"ok": false, "error": format!("slack api error: {err}"), "response": body_json}),
        );
    }

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

    // Slack URL verification challenge — must respond with the challenge value.
    // Sent when setting Event Subscriptions or Interactivity Request URL.
    if body_val.get("type").and_then(Value::as_str) == Some("url_verification") {
        let challenge = body_val
            .get("challenge")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let out = HttpOutV1 {
            status: 200,
            headers: vec![],
            body_b64: STANDARD.encode(challenge.as_bytes()),
            events: vec![],
        };
        return http_out_v1_bytes(&out);
    }

    // Slack interactive payloads (button clicks) come as URL-encoded `payload=<json>`
    // or directly as JSON with `type: "block_actions"`.
    // Also check for URL-encoded form body.
    let interactive_payload =
        if body_val.get("type").and_then(Value::as_str) == Some("block_actions") {
            Some(body_val.clone())
        } else {
            // Try URL-encoded: body may be "payload=%7B..." raw text.
            let body_str = String::from_utf8(body_bytes.clone()).unwrap_or_default();
            if let Some(payload) = body_str.strip_prefix("payload=") {
                let decoded = urldecode(payload);
                serde_json::from_str::<Value>(&decoded)
                    .ok()
                    .filter(|v| v.get("type").and_then(Value::as_str) == Some("block_actions"))
            } else {
                None
            }
        };

    // Handle view_submission — Slack modal form submitted.
    let view_submission_payload =
        if body_val.get("type").and_then(Value::as_str) == Some("view_submission") {
            Some(body_val.clone())
        } else {
            let body_str = String::from_utf8(body_bytes.clone()).unwrap_or_default();
            if let Some(payload) = body_str.strip_prefix("payload=") {
                let decoded = urldecode(payload);
                serde_json::from_str::<Value>(&decoded)
                    .ok()
                    .filter(|v| v.get("type").and_then(Value::as_str) == Some("view_submission"))
            } else {
                None
            }
        };

    if let Some(submission) = view_submission_payload {
        return handle_view_submission(&submission);
    }

    if let Some(interactive) = interactive_payload {
        // Handle block_actions — Slack button click (AC Action.Submit mapped to Slack button).
        let actions = interactive
            .get("actions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let channel = interactive
            .get("channel")
            .and_then(|v| v.get("id"))
            .and_then(Value::as_str)
            .map(|s| s.to_string());
        let sender = interactive
            .get("user")
            .and_then(|v| v.get("id"))
            .and_then(Value::as_str)
            .map(|s| s.to_string());

        // Extract routing info from first action's value.
        let first_action = actions.first().cloned().unwrap_or(Value::Null);
        let action_value_str = first_action
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let action_id = first_action
            .get("action_id")
            .and_then(Value::as_str)
            .unwrap_or_default();

        // Check if this button triggers a modal (AC input fields present).
        let parsed_action_val = serde_json::from_str::<Value>(action_value_str).ok();
        let is_modal = parsed_action_val
            .as_ref()
            .and_then(|v| v.get("ac_modal"))
            .and_then(Value::as_bool)
            .unwrap_or(false);

        if is_modal {
            let trigger_id = interactive
                .get("trigger_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !trigger_id.is_empty() {
                // Retrieve input specs from message metadata (not button value).
                let msg_metadata_inputs = interactive
                    .get("message")
                    .and_then(|m| m.get("metadata"))
                    .and_then(|m| m.get("event_payload"))
                    .and_then(|p| p.get("inputs"))
                    .cloned()
                    .unwrap_or(json!([]));
                // Merge: action data + input specs for the modal builder.
                let mut modal_data = parsed_action_val.clone().unwrap_or(json!({}));
                if let Some(obj) = modal_data.as_object_mut() {
                    obj.insert("ac_modal_inputs".into(), msg_metadata_inputs);
                }
                return open_slack_modal(trigger_id, &modal_data, channel.as_deref());
            }
        }

        // Try to parse action value as JSON for routeToCardId.
        let (_route_to_card, _card_id, action_text) =
            if let Ok(val) = serde_json::from_str::<Value>(action_value_str) {
                let rtc = val
                    .get("routeToCardId")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let cid = val
                    .get("cardId")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let text = if !rtc.is_empty() {
                    format!("[card:{rtc}]")
                } else if !cid.is_empty() {
                    format!("[action:{cid}]")
                } else {
                    format!("[action:{action_id}]")
                };
                (rtc, cid, text)
            } else {
                (
                    String::new(),
                    String::new(),
                    format!("[action:{action_id}]"),
                )
            };

        let mut envelope = build_slack_envelope(action_text, channel.clone(), sender);
        // Forward ALL Action.Submit data fields to metadata for MCP routing.
        if let Ok(val) = serde_json::from_str::<Value>(action_value_str)
            && let Some(obj) = val.as_object()
        {
            for (k, v) in obj {
                let s = match v {
                    Value::String(s) => s.clone(),
                    _ => v.to_string(),
                };
                envelope.metadata.insert(k.clone(), s);
            }
        }
        envelope
            .metadata
            .insert("slack.action_id".into(), action_id.to_string());
        envelope
            .metadata
            .insert("slack.action_value".into(), action_value_str.to_string());

        let normalized = json!({
            "ok": true,
            "event": interactive,
            "channel": channel,
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

    // Drop Slack retries — only process the first delivery.
    if is_slack_retry(&request) {
        let out = HttpOutV1 {
            status: 200,
            headers: Vec::new(),
            body_b64: STANDARD.encode(b"ok"),
            events: vec![],
        };
        return http_out_v1_bytes(&out);
    }

    // Slack Events API: {"type":"event_callback","event":{...}}
    // Legacy/generic:   {"body":{...}}
    // Flat:             {"text":"...","channel":"..."}
    let payload = body_val
        .get("event")
        .or_else(|| body_val.get("body"))
        .cloned()
        .unwrap_or_else(|| body_val.clone());

    // Skip bot messages to prevent echo loops.
    if is_bot_message(&payload) {
        let out = HttpOutV1 {
            status: 200,
            headers: Vec::new(),
            body_b64: STANDARD.encode(b"ok"),
            events: vec![],
        };
        return http_out_v1_bytes(&out);
    }

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

/// Check if the request is a Slack retry (X-Slack-Retry-Num header present).
fn is_slack_retry(request: &HttpInV1) -> bool {
    request.headers.iter().any(|h| {
        h.name.eq_ignore_ascii_case("x-slack-retry-num")
            || h.name.eq_ignore_ascii_case("X-Slack-Retry-Num")
    })
}

/// Check if the event payload is from a bot (prevents echo loops).
fn is_bot_message(payload: &Value) -> bool {
    // bot_id field present on bot-authored messages
    if payload.get("bot_id").is_some_and(|v| !v.is_null()) {
        return true;
    }
    // subtype "bot_message" is another indicator
    if payload.get("subtype").and_then(Value::as_str) == Some("bot_message") {
        return true;
    }
    false
}

pub(crate) fn render_plan(input_json: &[u8]) -> Vec<u8> {
    render_plan_common(
        input_json,
        &RenderPlanConfig {
            capabilities: PlannerCapabilities {
                supports_adaptive_cards: false,
                supports_markdown: true,
                supports_html: false,
                supports_images: false,
                supports_buttons: false,
                max_text_len: Some(40_000),
                max_payload_bytes: None,
            },
            default_summary: "slack message",
        },
    )
}

pub(crate) fn encode_op(input_json: &[u8]) -> Vec<u8> {
    use provider_common::helpers::extract_ac_summary;

    let encode_message = match decode_encode_message(input_json) {
        Ok(value) => value,
        Err(err) => return encode_error(&err),
    };
    let channel = encode_message
        .to
        .first()
        .map(|d| d.id.clone())
        .unwrap_or_default();
    if channel.is_empty() {
        return encode_error("destination (to) required");
    }

    // If the message carries an Adaptive Card, convert to Slack Block Kit.
    let ac_result = encode_message
        .metadata
        .get("adaptive_card")
        .and_then(|ac_raw| ac_to_slack_blocks(ac_raw));

    let text = if ac_result.is_some() {
        // Blocks present — text is the plain-text fallback for notifications.
        let caps = PlannerCapabilities {
            supports_adaptive_cards: false,
            supports_markdown: true,
            supports_html: false,
            supports_images: false,
            supports_buttons: false,
            max_text_len: Some(40_000),
            max_payload_bytes: None,
        };
        encode_message
            .metadata
            .get("adaptive_card")
            .and_then(|ac_raw| extract_ac_summary(ac_raw, &caps))
            .or_else(|| encode_message.text.clone().filter(|t| !t.trim().is_empty()))
            .unwrap_or_else(|| "slack universal payload".to_string())
    } else {
        encode_message
            .text
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| "slack universal payload".to_string())
    };

    let url = format!("{}/chat.postMessage", DEFAULT_API_BASE);
    let mut body = json!({
        "channel": channel,
        "text": text,
    });
    if let Some(ref result) = ac_result {
        body.as_object_mut()
            .unwrap()
            .insert("blocks".into(), Value::Array(result.blocks.clone()));
        // Store modal input specs in Slack message metadata (not in button value)
        // so they can be retrieved when a modal-trigger button is clicked.
        if !result.modal_inputs.is_empty() {
            body.as_object_mut().unwrap().insert(
                "metadata".into(),
                json!({
                    "event_type": "ac_modal_inputs",
                    "event_payload": {
                        "inputs": result.modal_inputs
                    }
                }),
            );
        }
    }
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
    // Slack returns HTTP 200 even on errors — check the JSON body.
    let body = resp.body.unwrap_or_default();
    let body_json: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    if body_json.get("ok").and_then(Value::as_bool) == Some(false) {
        let err = body_json
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("unknown slack error");
        return send_payload_error(&format!("slack api error: {err}"), false);
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

// ─── Adaptive Card → Slack Block Kit converter ──────────────────────────

/// Convert an Adaptive Card JSON string into Slack Block Kit blocks.
///
/// Maps AC elements to their best Slack-native representation:
/// - TextBlock (bold/heading) → `header` block (max 150 chars)
/// - TextBlock (normal) → `section` block with mrkdwn
/// - RichTextBlock → `section` with mrkdwn formatting
/// - Image/ImageSet → `image` block
/// - FactSet → `section` with `fields` array
/// - ColumnSet → `section` with `fields`
/// - Container → recursive processing
/// - ActionSet + top-level actions → `actions` block with buttons
/// - Table → `section` with preformatted code block
/// - Input.Text → collected for Slack modal (opened on Action.Submit click)
///   Result of converting an AC card to Slack blocks.
struct SlackBlocksResult {
    blocks: Vec<Value>,
    /// Input field specs for modal rendering (empty if no inputs).
    modal_inputs: Vec<Value>,
}

fn ac_to_slack_blocks(ac_raw: &str) -> Option<SlackBlocksResult> {
    let ac: Value = serde_json::from_str(ac_raw).ok()?;
    let body = ac.get("body").and_then(Value::as_array);
    let top_actions = ac.get("actions").and_then(Value::as_array);

    let mut blocks: Vec<Value> = Vec::new();
    let mut actions: Vec<Value> = Vec::new();

    // Collect input fields (Input.Text + Input.ChoiceSet) for modal support.
    let mut input_fields: Vec<Value> = Vec::new();
    if let Some(body) = body {
        collect_ac_input_fields(body, &mut input_fields);
    }
    let has_modal = !input_fields.is_empty();

    if let Some(body) = body {
        for element in body {
            ac_element_to_blocks(element, &mut blocks, &mut actions, has_modal);
        }
    }
    if let Some(top_actions) = top_actions {
        collect_slack_actions(top_actions, &mut actions);
    }

    // If there are input fields, mark Action.Submit buttons as modal triggers.
    // Input specs are NOT embedded in button value (too large) — they go in message metadata.
    if has_modal {
        inject_modal_metadata(&mut actions);
    }

    // Add actions block if any buttons were collected.
    if !actions.is_empty() {
        // Slack max 25 elements per actions block.
        let capped: Vec<Value> = actions.into_iter().take(25).collect();
        blocks.push(json!({
            "type": "actions",
            "elements": capped
        }));
    }

    if blocks.is_empty() {
        return None;
    }

    // Slack max 50 blocks per message.
    blocks.truncate(50);
    Some(SlackBlocksResult {
        blocks,
        modal_inputs: input_fields,
    })
}

/// Recursively collect input field specs (Input.Text + Input.ChoiceSet) from an AC body
/// for rendering in a Slack modal instead of inline in the message.
fn collect_ac_input_fields(elements: &[Value], inputs: &mut Vec<Value>) {
    for element in elements {
        let etype = element
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match etype {
            "Input.Text" => {
                let id = element.get("id").and_then(Value::as_str).unwrap_or("input");
                let label = element
                    .get("label")
                    .and_then(Value::as_str)
                    .or_else(|| element.get("placeholder").and_then(Value::as_str))
                    .unwrap_or(id);
                let placeholder = element
                    .get("placeholder")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let is_required = element
                    .get("isRequired")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let is_multiline = element
                    .get("isMultiline")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                inputs.push(json!({
                    "input_type": "text",
                    "id": id,
                    "label": label,
                    "placeholder": placeholder,
                    "required": is_required,
                    "multiline": is_multiline,
                }));
            }
            "Input.ChoiceSet" => {
                let id = element
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("choice");
                let label = element
                    .get("label")
                    .and_then(Value::as_str)
                    .or_else(|| element.get("placeholder").and_then(Value::as_str))
                    .unwrap_or(id);
                let placeholder = element
                    .get("placeholder")
                    .and_then(Value::as_str)
                    .unwrap_or("Select");
                let is_required = element
                    .get("isRequired")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let is_multi = element
                    .get("isMultiSelect")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let choices = element
                    .get("choices")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                inputs.push(json!({
                    "input_type": "choice",
                    "id": id,
                    "label": label,
                    "placeholder": placeholder,
                    "required": is_required,
                    "multi": is_multi,
                    "choices": choices,
                }));
            }
            "Container" => {
                if let Some(items) = element.get("items").and_then(Value::as_array) {
                    collect_ac_input_fields(items, inputs);
                }
            }
            "ColumnSet" => {
                if let Some(cols) = element.get("columns").and_then(Value::as_array) {
                    for col in cols {
                        if let Some(items) = col.get("items").and_then(Value::as_array) {
                            collect_ac_input_fields(items, inputs);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// Mark Action.Submit buttons as modal triggers by injecting `ac_modal: true`
/// into the button value. Input specs are stored in message metadata instead
/// of button value to avoid exceeding Slack's 2000 char limit.
fn inject_modal_metadata(actions: &mut [Value]) {
    for action in actions.iter_mut() {
        let is_url_button = action.get("url").is_some();
        if is_url_button {
            continue;
        }
        let existing_value = action.get("value").and_then(Value::as_str).unwrap_or("{}");
        let mut val: Value = serde_json::from_str(existing_value).unwrap_or(json!({}));
        if let Some(obj) = val.as_object_mut() {
            obj.insert("ac_modal".into(), json!(true));
        }
        if let Some(obj) = action.as_object_mut() {
            obj.insert("value".into(), Value::String(val.to_string()));
        }
    }
}

/// Convert AC markdown to Slack mrkdwn: `**bold**` → `*bold*`, `[text](url)` → `<url|text>`.
fn ac_markdown_to_slack(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        // **bold** → *bold*
        if i + 1 < chars.len()
            && chars[i] == '*'
            && chars[i + 1] == '*'
            && let Some(end) = chars[i + 2..]
                .windows(2)
                .position(|w| w[0] == '*' && w[1] == '*')
        {
            let inner: String = chars[i + 2..i + 2 + end].iter().collect();
            out.push('*');
            out.push_str(&inner);
            out.push('*');
            i += 4 + end;
            continue;
        }
        // [text](url) → <url|text>
        if chars[i] == '['
            && let Some(close_bracket) = chars[i + 1..].iter().position(|&c| c == ']')
        {
            let cb = i + 1 + close_bracket;
            if cb + 1 < chars.len()
                && chars[cb + 1] == '('
                && let Some(close_paren) = chars[cb + 2..].iter().position(|&c| c == ')')
            {
                let link_text: String = chars[i + 1..cb].iter().collect();
                let url: String = chars[cb + 2..cb + 2 + close_paren].iter().collect();
                out.push('<');
                out.push_str(&url);
                out.push('|');
                out.push_str(&link_text);
                out.push('>');
                i = cb + 3 + close_paren;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Extract all text from an AC element tree (for ColumnSet merging).
fn extract_texts_from_items(items: &[Value]) -> Vec<String> {
    let mut texts = Vec::new();
    for item in items {
        let etype = item.get("type").and_then(Value::as_str).unwrap_or_default();
        match etype {
            "TextBlock" => {
                if let Some(t) = item.get("text").and_then(Value::as_str) {
                    let t = t.trim();
                    if !t.is_empty() {
                        let is_bold = item
                            .get("weight")
                            .and_then(Value::as_str)
                            .is_some_and(|w| w.eq_ignore_ascii_case("bolder"));
                        let converted = ac_markdown_to_slack(t);
                        if is_bold && !converted.starts_with('*') {
                            texts.push(format!("*{converted}*"));
                        } else {
                            texts.push(converted);
                        }
                    }
                }
            }
            "Container" => {
                if let Some(sub) = item.get("items").and_then(Value::as_array) {
                    texts.extend(extract_texts_from_items(sub));
                }
            }
            _ => {
                if let Some(t) = item.get("text").and_then(Value::as_str)
                    && !t.trim().is_empty()
                {
                    texts.push(ac_markdown_to_slack(t.trim()));
                }
            }
        }
    }
    texts
}

/// Recursively convert an AC body element to Slack Block Kit blocks.
/// When `has_modal` is true, input fields (Input.Text, Input.ChoiceSet) are skipped
/// because they will be rendered inside a Slack modal instead.
fn ac_element_to_blocks(
    element: &Value,
    blocks: &mut Vec<Value>,
    actions: &mut Vec<Value>,
    has_modal: bool,
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
            let is_subtle = element
                .get("isSubtle")
                .and_then(Value::as_bool)
                .unwrap_or(false);

            let converted = ac_markdown_to_slack(text);

            if is_heading || size == "extralarge" {
                // Slack header block: plain_text, max 150 chars.
                // Strip mrkdwn chars for plain_text header.
                let plain: String = converted.replace('*', "").chars().take(150).collect();
                blocks.push(json!({
                    "type": "header",
                    "text": { "type": "plain_text", "text": plain, "emoji": true }
                }));
            } else if is_subtle || size == "small" {
                // Context block for subtle/small text — appears smaller and grayed.
                blocks.push(json!({
                    "type": "context",
                    "elements": [{ "type": "mrkdwn", "text": converted }]
                }));
            } else if is_bold || size == "large" {
                // Bold section.
                let bold = if converted.starts_with('*') {
                    converted
                } else {
                    format!("*{converted}*")
                };
                blocks.push(json!({
                    "type": "section",
                    "text": { "type": "mrkdwn", "text": bold }
                }));
            } else {
                blocks.push(json!({
                    "type": "section",
                    "text": { "type": "mrkdwn", "text": converted }
                }));
            }
        }

        "RichTextBlock" => {
            let inlines = element.get("inlines").and_then(Value::as_array);
            if let Some(inlines) = inlines {
                let mut mrkdwn = String::new();
                for inline in inlines {
                    let text = inline
                        .get("text")
                        .and_then(Value::as_str)
                        .or_else(|| inline.as_str())
                        .unwrap_or_default();
                    if text.is_empty() {
                        continue;
                    }
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
                    // Hyperlink
                    if let Some(url) = inline.get("selectAction").and_then(|a| {
                        if a.get("type").and_then(Value::as_str) == Some("Action.OpenUrl") {
                            a.get("url").and_then(Value::as_str)
                        } else {
                            None
                        }
                    }) {
                        s = format!("<{url}|{s}>");
                    }
                    mrkdwn.push_str(&s);
                }
                if !mrkdwn.is_empty() {
                    blocks.push(json!({
                        "type": "section",
                        "text": { "type": "mrkdwn", "text": mrkdwn }
                    }));
                }
            }
        }

        "Image" => {
            if let Some(url) = element.get("url").and_then(Value::as_str) {
                let alt = element
                    .get("altText")
                    .and_then(Value::as_str)
                    .unwrap_or("image");
                blocks.push(json!({
                    "type": "image",
                    "image_url": url,
                    "alt_text": alt
                }));
            }
        }

        "ImageSet" => {
            if let Some(imgs) = element.get("images").and_then(Value::as_array) {
                for img in imgs {
                    if let Some(url) = img.get("url").and_then(Value::as_str) {
                        let alt = img
                            .get("altText")
                            .and_then(Value::as_str)
                            .unwrap_or("image");
                        blocks.push(json!({
                            "type": "image",
                            "image_url": url,
                            "alt_text": alt
                        }));
                    }
                }
            }
        }

        "FactSet" => {
            if let Some(facts) = element.get("facts").and_then(Value::as_array) {
                // Slack section fields: max 10 fields, each max 2000 chars.
                let fields: Vec<Value> = facts
                    .iter()
                    .take(10)
                    .filter_map(|fact| {
                        let title = fact
                            .get("title")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let value = fact
                            .get("value")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        if title.is_empty() && value.is_empty() {
                            return None;
                        }
                        Some(json!({
                            "type": "mrkdwn",
                            "text": format!("*{title}*\n{value}")
                        }))
                    })
                    .collect();
                if !fields.is_empty() {
                    blocks.push(json!({
                        "type": "section",
                        "fields": fields
                    }));
                }
            }
        }

        "ColumnSet" => {
            if let Some(columns) = element.get("columns").and_then(Value::as_array) {
                // Try to merge icon+text columns into a single mrkdwn line.
                // Pattern: [auto-width emoji col] [stretch text col] → "emoji *title*\ndesc"
                let col_texts: Vec<Vec<String>> = columns
                    .iter()
                    .map(|col| {
                        col.get("items")
                            .and_then(Value::as_array)
                            .map(|items| extract_texts_from_items(items))
                            .unwrap_or_default()
                    })
                    .collect();

                if col_texts.len() == 2
                    && col_texts[0].len() == 1
                    && col_texts[0][0].chars().count() <= 3
                {
                    // Icon + text pattern — merge into single section.
                    let icon = &col_texts[0][0];
                    let text_parts = col_texts[1].join("\n");
                    let merged = format!("{icon}  {text_parts}");
                    blocks.push(json!({
                        "type": "section",
                        "text": { "type": "mrkdwn", "text": merged }
                    }));
                } else {
                    // General columns → section fields.
                    let mut fields: Vec<Value> = Vec::new();
                    for texts in &col_texts {
                        if !texts.is_empty() {
                            fields.push(json!({
                                "type": "mrkdwn",
                                "text": texts.join("\n")
                            }));
                        }
                    }
                    if !fields.is_empty() {
                        fields.truncate(10);
                        blocks.push(json!({
                            "type": "section",
                            "fields": fields
                        }));
                    }
                }

                // Convert Column selectAction / nested Container selectAction to Slack buttons.
                for col in columns {
                    collect_select_action(col, actions);
                    // Also check Container items inside each Column.
                    if let Some(items) = col.get("items").and_then(Value::as_array) {
                        for item in items {
                            if item.get("type").and_then(Value::as_str) == Some("Container") {
                                collect_select_action(item, actions);
                            }
                        }
                    }
                }
            }

            // ColumnSet-level selectAction.
            collect_select_action(element, actions);
        }

        "Container" => {
            // In AC, isVisible:false containers are toggled by Action.ToggleVisibility.
            // Slack has no toggle concept, so render hidden containers anyway — the user
            // needs to see the content that would normally be revealed by a toggle.

            let has_style = element
                .get("style")
                .and_then(Value::as_str)
                .is_some_and(|s| {
                    s == "accent"
                        || s == "emphasis"
                        || s == "good"
                        || s == "attention"
                        || s == "warning"
                });

            // Add divider before styled containers for visual separation.
            if has_style && !blocks.is_empty() {
                blocks.push(json!({"type": "divider"}));
            }

            if let Some(items) = element.get("items").and_then(Value::as_array) {
                for item in items {
                    ac_element_to_blocks(item, blocks, actions, has_modal);
                }
            }

            // Convert Container selectAction to Slack button.
            collect_select_action(element, actions);
        }

        "ActionSet" => {
            if let Some(action_list) = element.get("actions").and_then(Value::as_array) {
                collect_slack_actions(action_list, actions);
            }
        }

        "Input.Text" | "Input.ChoiceSet" => {
            // When modal is active, skip inline rendering — these will be in the modal.
            // Otherwise fall back to inline rendering.
            if has_modal {
                return;
            }
            if etype == "Input.Text" {
                let label = element
                    .get("label")
                    .and_then(Value::as_str)
                    .or_else(|| element.get("placeholder").and_then(Value::as_str));
                if let Some(label) = label {
                    blocks.push(json!({
                        "type": "context",
                        "elements": [{ "type": "mrkdwn", "text": format!("_{label}_") }]
                    }));
                }
            } else {
                // Input.ChoiceSet inline fallback (no modal).
                if let Some(choices) = element.get("choices").and_then(Value::as_array) {
                    let options: Vec<Value> = choices
                        .iter()
                        .take(100)
                        .filter_map(|c| {
                            let title = c.get("title").and_then(Value::as_str)?;
                            let value = c.get("value").and_then(Value::as_str).unwrap_or(title);
                            Some(json!({
                                "text": { "type": "plain_text", "text": title.chars().take(75).collect::<String>() },
                                "value": value.chars().take(75).collect::<String>()
                            }))
                        })
                        .collect();
                    if !options.is_empty() {
                        let input_id = element
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("choice");
                        let placeholder = element
                            .get("placeholder")
                            .and_then(Value::as_str)
                            .unwrap_or("Select");
                        let is_multi = element
                            .get("isMultiSelect")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        let select_type = if is_multi {
                            "multi_static_select"
                        } else {
                            "static_select"
                        };
                        let select = json!({
                            "type": select_type,
                            "action_id": format!("ac_input_{input_id}"),
                            "placeholder": { "type": "plain_text", "text": placeholder.chars().take(150).collect::<String>() },
                            "options": options
                        });
                        let label = element
                            .get("label")
                            .and_then(Value::as_str)
                            .unwrap_or(placeholder);
                        blocks.push(json!({
                            "type": "section",
                            "text": { "type": "mrkdwn", "text": format!("*{label}*") },
                            "accessory": select
                        }));
                    }
                }
            }
        }

        "Table" => {
            let rows = element.get("rows").and_then(Value::as_array);
            let columns = element.get("columns").and_then(Value::as_array);
            if let Some(rows) = rows {
                let mut lines = Vec::new();
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
                        lines.push(
                            headers
                                .iter()
                                .map(|h| format!("*{h}*"))
                                .collect::<Vec<_>>()
                                .join(" | "),
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
                        lines.push(cell_texts.join(" | "));
                    }
                }
                if !lines.is_empty() {
                    blocks.push(json!({
                        "type": "section",
                        "text": {
                            "type": "mrkdwn",
                            "text": format!("```\n{}\n```", lines.join("\n"))
                        }
                    }));
                }
            }
        }

        _ => {}
    }
}

/// Extract a human-readable label from a Container/Column's child items.
/// Recurses into ColumnSet > Column > items and Container > items to find
/// a suitable TextBlock label.
fn label_from_items(items: &[Value]) -> String {
    // First try: bold TextBlock at this level
    for item in items {
        if item.get("type").and_then(Value::as_str) == Some("TextBlock")
            && item
                .get("weight")
                .and_then(Value::as_str)
                .is_some_and(|w| w.eq_ignore_ascii_case("bolder"))
            && let Some(text) = item.get("text").and_then(Value::as_str)
        {
            let t = text.trim();
            if !t.is_empty() {
                return t.chars().take(75).collect();
            }
        }
    }
    // Fallback: first non-empty TextBlock at this level
    for item in items {
        if item.get("type").and_then(Value::as_str) == Some("TextBlock")
            && let Some(text) = item.get("text").and_then(Value::as_str)
        {
            let t = text.trim();
            if !t.is_empty() {
                return t.chars().take(75).collect();
            }
        }
    }
    // Recurse into nested structures (ColumnSet > Column, Container)
    for item in items {
        let etype = item.get("type").and_then(Value::as_str).unwrap_or_default();
        let nested = match etype {
            "ColumnSet" => item.get("columns").and_then(Value::as_array).map(|cols| {
                cols.iter()
                    .filter_map(|col| col.get("items").and_then(Value::as_array))
                    .flatten()
                    .cloned()
                    .collect::<Vec<Value>>()
            }),
            "Container" => item.get("items").and_then(Value::as_array).cloned(),
            _ => None,
        };
        if let Some(children) = nested {
            let label = label_from_items(&children);
            if !label.is_empty() {
                return label;
            }
        }
    }
    String::new()
}

/// If an AC element has a `selectAction`, convert it to a Slack button.
fn collect_select_action(element: &Value, actions: &mut Vec<Value>) {
    let sa = match element.get("selectAction") {
        Some(sa) => sa,
        None => return,
    };
    let atype = sa.get("type").and_then(Value::as_str).unwrap_or_default();
    if atype != "Action.Submit" && atype != "Action.Execute" {
        return;
    }

    // Derive button label: try selectAction.title first, then child items, then fallback.
    let sa_title = sa
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let label = if !sa_title.is_empty() {
        sa_title
    } else {
        let items = element
            .get("items")
            .and_then(Value::as_array)
            .map(|a| a.as_slice())
            .unwrap_or(&[]);
        label_from_items(items)
    };
    // Fallback: derive from data keys or use generic label.
    let label = if label.is_empty() {
        sa.get("data")
            .and_then(|d| d.get("routeToCardId").and_then(Value::as_str))
            .unwrap_or("Select")
            .to_string()
    } else {
        label
    };

    let btn_text: String = label.chars().take(75).collect();
    let mut btn = json!({
        "type": "button",
        "text": { "type": "plain_text", "text": btn_text },
        "action_id": format!("ac_action_{}", actions.len())
    });
    if let Some(data) = sa.get("data") {
        btn.as_object_mut()
            .unwrap()
            .insert("value".into(), Value::String(data.to_string()));
    }
    actions.push(btn);
}

/// Collect AC actions into Slack button elements.
fn collect_slack_actions(action_list: &[Value], actions: &mut Vec<Value>) {
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
        // Slack button text max 75 chars.
        let btn_text: String = title.chars().take(75).collect();
        match atype {
            "Action.OpenUrl" => {
                let url = action.get("url").and_then(Value::as_str).unwrap_or("");
                actions.push(json!({
                    "type": "button",
                    "text": { "type": "plain_text", "text": btn_text },
                    "url": url,
                    "action_id": format!("ac_url_{}", actions.len())
                }));
            }
            _ => {
                // Action.Submit, Action.Execute, etc. → callback button.
                // Include AC action data (routeToCardId, cardId, etc.) in button value.
                let mut btn = json!({
                    "type": "button",
                    "text": { "type": "plain_text", "text": btn_text },
                    "action_id": format!("ac_action_{}", actions.len())
                });
                if let Some(data) = action.get("data") {
                    btn.as_object_mut()
                        .unwrap()
                        .insert("value".into(), Value::String(data.to_string()));
                }
                actions.push(btn);
            }
        }
    }
}

// ─── Slack Modal support (AC inputs → views.open) ───────────────────────

/// Build a Slack modal view from AC input field specs (Input.Text + Input.ChoiceSet)
/// and open it via the Slack `views.open` API using the `trigger_id` from the interaction.
fn open_slack_modal(trigger_id: &str, action_data: &Value, channel: Option<&str>) -> Vec<u8> {
    let inputs = action_data
        .get("ac_modal_inputs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    // Build modal input blocks from the AC input specs.
    let mut modal_blocks: Vec<Value> = Vec::new();
    for input in &inputs {
        let id = input.get("id").and_then(Value::as_str).unwrap_or("input");
        let label = input.get("label").and_then(Value::as_str).unwrap_or(id);
        let placeholder = input
            .get("placeholder")
            .and_then(Value::as_str)
            .unwrap_or("");
        let is_required = input
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let input_type = input
            .get("input_type")
            .and_then(Value::as_str)
            .unwrap_or("text");

        let element = match input_type {
            "choice" => {
                // Build static_select / multi_static_select from choices.
                let choices = input
                    .get("choices")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let options: Vec<Value> = choices
                    .iter()
                    .take(100)
                    .filter_map(|c| {
                        let title = c.get("title").and_then(Value::as_str)?;
                        let value = c.get("value").and_then(Value::as_str).unwrap_or(title);
                        Some(json!({
                            "text": { "type": "plain_text", "text": title.chars().take(75).collect::<String>() },
                            "value": value.chars().take(75).collect::<String>()
                        }))
                    })
                    .collect();
                if options.is_empty() {
                    continue;
                }
                let is_multi = input.get("multi").and_then(Value::as_bool).unwrap_or(false);
                let select_type = if is_multi {
                    "multi_static_select"
                } else {
                    "static_select"
                };
                let mut el = json!({
                    "type": select_type,
                    "action_id": id,
                    "options": options,
                });
                if !placeholder.is_empty() {
                    el.as_object_mut().unwrap().insert(
                        "placeholder".into(),
                        json!({"type": "plain_text", "text": placeholder.chars().take(150).collect::<String>()}),
                    );
                }
                el
            }
            _ => {
                // plain_text_input for Input.Text
                let is_multiline = input
                    .get("multiline")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let mut el = json!({
                    "type": "plain_text_input",
                    "action_id": id,
                    "multiline": is_multiline,
                });
                if !placeholder.is_empty() {
                    el.as_object_mut().unwrap().insert(
                        "placeholder".into(),
                        json!({"type": "plain_text", "text": placeholder.chars().take(150).collect::<String>()}),
                    );
                }
                el
            }
        };

        let block = json!({
            "type": "input",
            "block_id": format!("ac_input_{id}"),
            "optional": !is_required,
            "label": { "type": "plain_text", "text": label.chars().take(48).collect::<String>() },
            "element": element,
        });
        modal_blocks.push(block);
    }

    if modal_blocks.is_empty() {
        // Fallback: no inputs found, just ack.
        let out = HttpOutV1 {
            status: 200,
            headers: Vec::new(),
            body_b64: STANDARD.encode(b"{}"),
            events: vec![],
        };
        return http_out_v1_bytes(&out);
    }

    // Preserve the original action data (minus modal fields) as private_metadata
    // so we can forward it when the modal is submitted.
    let mut forward_data = action_data.clone();
    if let Some(obj) = forward_data.as_object_mut() {
        obj.remove("ac_modal");
        obj.remove("ac_modal_inputs");
        if let Some(ch) = channel {
            obj.insert("_channel".into(), Value::String(ch.to_string()));
        }
    }
    let private_metadata = forward_data.to_string();

    // Build the modal view.
    let title_text = action_data
        .get("routeToCardId")
        .and_then(Value::as_str)
        .unwrap_or("Input Required");
    let view = json!({
        "type": "modal",
        "title": { "type": "plain_text", "text": title_text.chars().take(24).collect::<String>() },
        "submit": { "type": "plain_text", "text": "Submit" },
        "close": { "type": "plain_text", "text": "Cancel" },
        "private_metadata": private_metadata.chars().take(3000).collect::<String>(),
        "blocks": modal_blocks,
    });

    let api_body = json!({
        "trigger_id": trigger_id,
        "view": view,
    });

    // Call views.open API.
    let token = match get_secret_string(DEFAULT_BOT_TOKEN_KEY) {
        Ok(t) => t,
        Err(err) => {
            return http_out_error(500, &format!("cannot open modal: secret error: {err}"));
        }
    };
    let api_url = format!("{}/views.open", DEFAULT_API_BASE);
    let req_body = serde_json::to_vec(&api_body).unwrap_or_else(|_| b"{}".to_vec());
    let request = client::Request {
        method: "POST".to_string(),
        url: api_url,
        headers: vec![
            ("Content-Type".into(), "application/json".into()),
            ("Authorization".into(), format!("Bearer {token}")),
        ],
        body: Some(req_body),
    };
    let resp = client::send(&request, None, None);
    if let Err(err) = &resp {
        return http_out_error(500, &format!("views.open transport error: {}", err.message));
    }
    let resp = resp.unwrap();
    if resp.status < 200 || resp.status >= 300 {
        return http_out_error(500, &format!("views.open returned status {}", resp.status));
    }

    // Return 200 with no events — the modal submission will arrive later.
    let out = HttpOutV1 {
        status: 200,
        headers: Vec::new(),
        body_b64: STANDARD.encode(b""),
        events: vec![],
    };
    http_out_v1_bytes(&out)
}

/// Handle `view_submission` — user submitted a Slack modal that was opened
/// from an AC Input.Text button. Extract input values, merge with the
/// original action data (from private_metadata), and create an envelope.
fn handle_view_submission(submission: &Value) -> Vec<u8> {
    let user = submission
        .get("user")
        .and_then(|v| v.get("id"))
        .and_then(Value::as_str)
        .map(|s| s.to_string());

    // Parse the private_metadata to recover the original action data.
    let private_metadata = submission
        .get("view")
        .and_then(|v| v.get("private_metadata"))
        .and_then(Value::as_str)
        .unwrap_or("{}");
    let mut action_data: Value = serde_json::from_str(private_metadata).unwrap_or(json!({}));

    // Extract channel from preserved metadata.
    let channel = action_data
        .get("_channel")
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    if let Some(obj) = action_data.as_object_mut() {
        obj.remove("_channel");
    }

    // Extract input values from the modal state.
    let state_values = submission
        .get("view")
        .and_then(|v| v.get("state"))
        .and_then(|v| v.get("values"))
        .cloned()
        .unwrap_or(json!({}));

    // Flatten: state.values is { block_id: { action_id: { type, value/selected_option } } }
    let mut input_values: BTreeMap<String, String> = BTreeMap::new();
    if let Some(blocks) = state_values.as_object() {
        for (_block_id, actions) in blocks {
            if let Some(actions_obj) = actions.as_object() {
                for (action_id, action_val) in actions_obj {
                    // plain_text_input → "value"
                    // static_select → "selected_option.value"
                    // multi_static_select → "selected_options[].value"
                    let value = if let Some(v) = action_val.get("value").and_then(Value::as_str) {
                        v.to_string()
                    } else if let Some(opt) = action_val.get("selected_option") {
                        opt.get("value")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string()
                    } else if let Some(opts) =
                        action_val.get("selected_options").and_then(Value::as_array)
                    {
                        opts.iter()
                            .filter_map(|o| o.get("value").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                            .join(",")
                    } else {
                        String::new()
                    };
                    if !value.is_empty() {
                        input_values.insert(action_id.clone(), value);
                    }
                }
            }
        }
    }

    // Merge input values into action_data.
    if let Some(obj) = action_data.as_object_mut() {
        for (k, v) in &input_values {
            obj.insert(k.clone(), Value::String(v.clone()));
        }
    }

    // Build the action text for the envelope.
    let route_to_card = action_data
        .get("routeToCardId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let action_text = if !route_to_card.is_empty() {
        format!("[card:{route_to_card}]")
    } else {
        "[modal:submit]".to_string()
    };

    let mut envelope = build_slack_envelope(action_text, channel.clone(), user);
    // Forward all action data fields to metadata.
    if let Some(obj) = action_data.as_object() {
        for (k, v) in obj {
            let s = match v {
                Value::String(s) => s.clone(),
                _ => v.to_string(),
            };
            envelope.metadata.insert(k.clone(), s);
        }
    }
    envelope
        .metadata
        .insert("slack.modal_submission".into(), "true".to_string());
    // Also insert raw input values for easy access.
    for (k, v) in &input_values {
        envelope.metadata.insert(format!("input.{k}"), v.clone());
    }

    // Return empty response body to close the modal (Slack requires empty or
    // `{"response_action":"clear"}` to dismiss). Events are processed by the operator.
    let out = HttpOutV1 {
        status: 200,
        headers: Vec::new(),
        body_b64: STANDARD.encode(b""),
        events: vec![envelope],
    };
    http_out_v1_bytes(&out)
}

/// Simple percent-decode for URL-encoded strings.
fn urldecode(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(ch) = chars.next() {
        if ch == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                result.push(byte as char);
            } else {
                result.push('%');
                result.push_str(&hex);
            }
        } else if ch == '+' {
            result.push(' ');
        } else {
            result.push(ch);
        }
    }
    result
}
