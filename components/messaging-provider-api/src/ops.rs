//! JSON op dispatch: ingest_http, send, poll_events and the egress pipeline.

use crate::host::{HostEnv, HostStore, load_config};
use api_channel_core::egress::{encode_payload, send_input};
use api_channel_core::http::{HttpRequest, Inbound};
use api_channel_core::ingest::ingest_http as core_ingest;
use api_channel_core::log::run_poll_events;
use api_channel_core::send::run_send;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use greentic_types::messaging::universal_dto::{Header, HttpInV1, HttpOutV1, ProviderPayloadV1};
use greentic_types::{Actor, ChannelMessageEnvelope, EnvId, MessageMetadata, TenantCtx, TenantId};
use provider_common::helpers::{
    RenderPlanConfig, decode_encode_message, encode_error, json_bytes, render_plan_common,
    resolve_adaptive_card, send_payload_dispatch,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const PROVIDER_TYPE: &str = "messaging.api";

pub fn dispatch(op: &str, input: &[u8]) -> Vec<u8> {
    match op {
        "ingest_http" | "ingest-http" => ingest_http(input),
        "send" | "run" => send(input),
        "poll_events" | "poll-events" => run_poll_events(&mut HostStore, input),
        "render_plan" | "render-plan" => render_plan(input),
        "encode" => encode(input),
        "send_payload" | "send-payload" => send_payload(input),
        other => json_bytes(&json!({"ok": false, "error": format!("unsupported op: {other}")})),
    }
}

/// The tenant is the path segment after `api` (`/v1/messaging/api/{tenant}/..`).
pub(crate) fn tenant_from_path(path: &str) -> Option<&str> {
    let mut segments = path.split('/').filter(|s| !s.is_empty());
    segments.find(|s| *s == "api")?;
    segments.next().filter(|t| *t != "messages")
}

pub(crate) fn envelope(inbound: &Inbound, tenant: &str) -> Result<ChannelMessageEnvelope, String> {
    let env = EnvId::try_from("default").map_err(|e| format!("env id: {e}"))?;
    let tenant_id = TenantId::try_from(tenant)
        .or_else(|_| TenantId::try_from("default"))
        .map_err(|e| format!("tenant id: {e}"))?;
    let mut metadata = MessageMetadata::new();
    metadata.insert("universal".into(), "true".into());
    metadata.insert("tenant".into(), tenant.to_string());
    metadata.insert("conversation_id".into(), inbound.conversation_id.clone());
    metadata.insert("message_id".into(), inbound.message_id.clone());
    metadata.insert("route".into(), inbound.conversation_id.clone());
    // The caller holds the channel API key; no individual user is verified.
    metadata.insert("user_verified".into(), "false".into());
    if let Some(card) = &inbound.card {
        metadata.insert("adaptive_card".into(), card.to_string());
    }
    Ok(ChannelMessageEnvelope {
        id: format!("api-{}", inbound.message_id),
        tenant: TenantCtx::new(env, tenant_id),
        channel: inbound.conversation_id.clone(),
        session_id: inbound.conversation_id.clone(),
        reply_scope: None,
        from: Some(Actor {
            id: format!("api:{}", inbound.conversation_id),
            kind: Some("user".into()),
        }),
        to: Vec::new(),
        correlation_id: None,
        text: inbound.text.clone(),
        attachments: Vec::new(),
        metadata,
        extensions: BTreeMap::new(),
    })
}

fn ingest_http(input: &[u8]) -> Vec<u8> {
    let Ok(request) = serde_json::from_slice::<HttpInV1>(input) else {
        return json_bytes(&json!({"ok": false, "error": "invalid ingest_http input"}));
    };
    let body = STANDARD
        .decode(request.body_b64.as_bytes())
        .unwrap_or_default();
    let config = load_config(request.config.as_ref());
    let core_request = HttpRequest {
        method: request.method.clone(),
        path: request.path.clone(),
        query: request.query.clone().unwrap_or_default(),
        headers: request
            .headers
            .iter()
            .map(|h| (h.name.clone(), h.value.clone()))
            .collect(),
        body,
    };
    let result = core_ingest(&mut HostStore, &mut HostEnv, &config, &core_request);

    let mut events = Vec::new();
    if let Some(inbound) = &result.inbound {
        let tenant = tenant_from_path(&request.path).unwrap_or("default");
        match envelope(inbound, tenant) {
            Ok(env) => events.push(env),
            Err(_) => {
                return http_out(&HttpOutV1 {
                    status: 500,
                    headers: json_headers(),
                    body_b64: STANDARD.encode(br#"{"error":{"code":"internal_error"}}"#),
                    events: Vec::new(),
                });
            }
        }
    }
    http_out(&HttpOutV1 {
        status: result.response.status,
        headers: json_headers(),
        body_b64: STANDARD.encode(&result.response.body),
        events,
    })
}

fn json_headers() -> Vec<Header> {
    vec![
        Header {
            name: "Content-Type".into(),
            value: "application/json".into(),
        },
        Header {
            name: "Cache-Control".into(),
            value: "no-store".into(),
        },
    ]
}

fn http_out(out: &HttpOutV1) -> Vec<u8> {
    json_bytes(out)
}

fn send(input: &[u8]) -> Vec<u8> {
    // The host may hand provider config beside the op input.
    let config_value = serde_json::from_slice::<Value>(input)
        .ok()
        .and_then(|v| v.get("config").cloned());
    let config = load_config(config_value.as_ref());
    run_send(&mut HostStore, &mut HostEnv, &config, input)
}

fn render_plan(input: &[u8]) -> Vec<u8> {
    // Cards travel as JSON and the client renders them, which is the
    // native-card tier; reuse the webchat capability row.
    let Some(capabilities) = provider_common::render::capabilities_for("webchat") else {
        return json_bytes(&json!({"ok": false, "error": "capabilities unavailable"}));
    };
    render_plan_common(
        input,
        &RenderPlanConfig {
            capabilities,
            default_summary: "api message",
        },
    )
}

fn encode(input: &[u8]) -> Vec<u8> {
    let message = match decode_encode_message(input) {
        Ok(message) => message,
        Err(err) => return encode_error(&err),
    };
    let failed = message.metadata.contains_key("error_kind");
    let awaiting = message
        .metadata
        .get("awaiting_input")
        .is_some_and(|v| v == "true");
    let payload = encode_payload(
        &message.session_id,
        message.text.as_deref(),
        resolve_adaptive_card(&message),
        failed,
        awaiting,
    );
    let body = serde_json::to_vec(&payload).unwrap_or_default();
    let payload = ProviderPayloadV1 {
        content_type: "application/json".to_string(),
        body_b64: STANDARD.encode(body),
        metadata: BTreeMap::new(),
    };
    json_bytes(&json!({"ok": true, "payload": payload}))
}

fn send_payload(input: &[u8]) -> Vec<u8> {
    send_payload_dispatch(input, PROVIDER_TYPE, |payload_bytes| {
        let payload: Value = serde_json::from_slice(payload_bytes).unwrap_or(Value::Null);
        let send_in = match send_input(&payload) {
            Ok(value) => value,
            Err(err) => return json_bytes(&json!({"ok": false, "error": err})),
        };
        let out = send(send_in.to_string().as_bytes());
        // Never forward host or provider text; the generic code is enough.
        let ok = serde_json::from_slice::<Value>(&out)
            .ok()
            .and_then(|v| v.get("ok").and_then(Value::as_bool))
            .unwrap_or(false);
        if ok {
            json_bytes(&json!({"ok": true}))
        } else {
            json_bytes(&json!({"ok": false, "error": "send failed"}))
        }
    })
}
