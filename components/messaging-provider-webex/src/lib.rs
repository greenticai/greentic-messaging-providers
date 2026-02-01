use base64::{Engine as _, engine::general_purpose};
use greentic_types::{
    Actor, ChannelMessageEnvelope, Destination, EnvId, MessageMetadata, TenantCtx, TenantId,
};
use messaging_universal_dto::{
    EncodeInV1, HttpInV1, HttpOutV1, ProviderPayloadV1, RenderPlanInV1, RenderPlanOutV1,
    SendPayloadInV1, SendPayloadResultV1,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

mod bindings {
    wit_bindgen::generate!({
        path: "wit/messaging-provider-webex",
        world: "messaging-provider-webex",
        generate_all
    });
}

use bindings::exports::greentic::provider_schema_core::schema_core_api::Guest;
use bindings::greentic::http::client;
use bindings::greentic::secrets_store::secrets_store;
use greentic_types::ProviderManifest;

const PROVIDER_TYPE: &str = "messaging.webex.bot";
const CONFIG_SCHEMA_REF: &str = "schemas/messaging/webex/public.config.schema.json";
const DEFAULT_API_BASE: &str = "https://webexapis.com/v1";
const DEFAULT_TOKEN_KEY: &str = "WEBEX_BOT_TOKEN";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderConfig {
    #[serde(default)]
    default_room_id: Option<String>,
    #[serde(default)]
    api_base_url: Option<String>,
}

struct Component;

impl Guest for Component {
    fn describe() -> Vec<u8> {
        let manifest = ProviderManifest {
            provider_type: PROVIDER_TYPE.to_string(),
            capabilities: vec![],
            ops: vec![
                "send".to_string(),
                "reply".to_string(),
                "ingest_http".to_string(),
                "render_plan".to_string(),
                "encode".to_string(),
                "send_payload".to_string(),
            ],
            config_schema_ref: Some(CONFIG_SCHEMA_REF.to_string()),
            state_schema_ref: None,
        };
        json_bytes(&manifest)
    }

    fn validate_config(config_json: Vec<u8>) -> Vec<u8> {
        match parse_config_bytes(&config_json) {
            Ok(cfg) => json_bytes(&json!({
                "ok": true,
                "config": {
                    "default_room_id": cfg.default_room_id,
                    "api_base_url": cfg.api_base_url.unwrap_or_else(|| DEFAULT_API_BASE.to_string()),
                }
            })),
            Err(err) => json_bytes(&json!({"ok": false, "error": err})),
        }
    }

    fn healthcheck() -> Vec<u8> {
        json_bytes(&json!({"status": "ok"}))
    }

    fn invoke(op: String, input_json: Vec<u8>) -> Vec<u8> {
        match op.as_str() {
            "send" => handle_send(&input_json),
            "reply" => handle_reply(&input_json),
            "ingest_http" => ingest_http(&input_json),
            "render_plan" => render_plan(&input_json),
            "encode" => encode_op(&input_json),
            "send_payload" => send_payload(&input_json),
            other => json_bytes(&json!({"ok": false, "error": format!("unsupported op: {other}")})),
        }
    }
}

bindings::exports::greentic::provider_schema_core::schema_core_api::__export_greentic_provider_schema_core_schema_core_api_1_0_0_cabi!(
    Component with_types_in bindings::exports::greentic::provider_schema_core::schema_core_api
);

fn handle_send(input_json: &[u8]) -> Vec<u8> {
    let parsed: Value = match serde_json::from_slice(input_json) {
        Ok(val) => val,
        Err(err) => {
            return json_bytes(&json!({"ok": false, "error": format!("invalid json: {err}")}));
        }
    };

    let envelope = match serde_json::from_value::<ChannelMessageEnvelope>(parsed.clone()) {
        Ok(env) => env,
        Err(err) => {
            return json_bytes(&json!({"ok": false, "error": format!("invalid envelope: {err}")}));
        }
    };

    if !envelope.attachments.is_empty() {
        return json_bytes(&json!({"ok": false, "error": "attachments not supported"}));
    }

    let cfg = match load_config(&parsed, Some(&envelope.metadata)) {
        Ok(cfg) => cfg,
        Err(err) => return json_bytes(&json!({"ok": false, "error": err})),
    };

    let text = envelope
        .text
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(ToString::to_string)
        .unwrap_or_default();
    if text.is_empty() {
        return json_bytes(&json!({"ok": false, "error": "text required"}));
    }

    let dest = match resolve_webex_destination(&envelope, &cfg) {
        Ok(dest) => dest,
        Err(err) if err == "destination required" => {
            return json_bytes(&destination_required_response());
        }
        Err(err) => return json_bytes(&json!({"ok": false, "error": err})),
    };

    let token = match secrets_store::get(DEFAULT_TOKEN_KEY) {
        Ok(Some(bytes)) => match String::from_utf8(bytes) {
            Ok(s) => s,
            Err(_) => return json_bytes(&json!({"ok": false, "error": "access_token not utf-8"})),
        },
        Ok(None) => {
            return json_bytes(
                &json!({"ok": false, "error": format!("missing secret: {}", DEFAULT_TOKEN_KEY)}),
            );
        }
        Err(e) => {
            return json_bytes(
                &json!({"ok": false, "error": format!("secret store error: {e:?}")}),
            );
        }
    };

    let api_base = cfg
        .api_base_url
        .unwrap_or_else(|| DEFAULT_API_BASE.to_string());
    let url = format!("{}/messages", api_base);
    let mut body = json!({ "text": text });
    eprintln!(
        "resolved destination -> room={:?}, personEmail={:?}, personId={:?}",
        dest.room_id, dest.person_email, dest.person_id
    );
    if let Some(room) = dest.room_id.clone() {
        body.as_object_mut()
            .expect("body object")
            .insert("roomId".into(), Value::String(room));
    }
    if let Some(person) = dest.person_email.clone() {
        body.as_object_mut()
            .expect("body object")
            .insert("toPersonEmail".into(), Value::String(person));
    } else if let Some(person_id) = dest.person_id.clone() {
        body.as_object_mut()
            .expect("body object")
            .insert("toPersonId".into(), Value::String(person_id));
    }

    let request = client::Request {
        method: "POST".into(),
        url,
        headers: vec![
            ("Content-Type".into(), "application/json".into()),
            ("Authorization".into(), format!("Bearer {token}")),
        ],
        body: Some(serde_json::to_vec(&body).unwrap_or_else(|_| b"{}".to_vec())),
    };

    if let Some(body_bytes) = &request.body {
        let body_text = String::from_utf8_lossy(body_bytes);
        eprintln!(
            "webex send request -> method={} url={} body={}",
            request.method, request.url, body_text,
        );
    }
    let request_method = request.method.clone();
    let request_url = request.url.clone();
    let request_body_text = request
        .body
        .as_ref()
        .map(|bytes| String::from_utf8_lossy(bytes).to_string())
        .unwrap_or_default();

    let resp = match client::send(&request, None, None) {
        Ok(resp) => resp,
        Err(err) => {
            return json_bytes(&json!({
                "ok": false,
                "error": format!("transport error: {} ({} {} body={})", err.message, request_method, request_url, request_body_text)
            }));
        }
    };

    if resp.status < 200 || resp.status >= 300 {
        let body = resp.body.clone().unwrap_or_default();
        let body_text = String::from_utf8_lossy(&body);
        eprintln!("webex send failure (status {}): {}", resp.status, body_text);
        return json_bytes(&json!({
            "ok": false,
            "error": format!("webex returned status {} for {} {} body={}", resp.status, request_method, request_url, body_text)
        }));
    }

    let body_bytes = resp.body.unwrap_or_default();
    let body_json: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    let msg_id = body_json
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("webex-message")
        .to_string();
    let provider_message_id = format!("webex:{msg_id}");

    json_bytes(&json!({
        "ok": true,
        "status": "sent",
        "provider_type": PROVIDER_TYPE,
        "message_id": msg_id,
        "provider_message_id": provider_message_id,
        "response": body_json
    }))
}

fn resolve_webex_destination(
    envelope: &ChannelMessageEnvelope,
    cfg: &ProviderConfig,
) -> Result<WebexDestination, String> {
    if let Some(dest) = envelope.to.iter().find(|dest| !dest.id.trim().is_empty()) {
        return map_webex_destination(dest);
    }

    if let Some(default_room) = cfg.default_room_id.clone() {
        if !default_room.trim().is_empty() {
            return Ok(WebexDestination {
                room_id: Some(default_room),
                person_email: None,
                person_id: None,
            });
        }
    }

    Err("destination required".to_string())
}

fn map_webex_destination(destination: &Destination) -> Result<WebexDestination, String> {
    let id = destination.id.trim();
    if id.is_empty() {
        return Err("destination required".to_string());
    }
    fn infer_kind(id: &str) -> Option<String> {
        if id.contains("/ROOM/") {
            Some("room".to_string())
        } else if id.contains("/PEOPLE/") {
            Some("personId".to_string())
        } else {
            None
        }
    }
    match destination.kind.as_deref() {
        Some("room") => Ok(WebexDestination {
            room_id: Some(destination.id.clone()),
            person_email: None,
            person_id: None,
        }),
        Some("personId") | Some("person_id") | Some("user") => Ok(WebexDestination {
            room_id: None,
            person_email: None,
            person_id: Some(destination.id.clone()),
        }),
        Some("personEmail") | Some("person_email") | Some("email") => Ok(WebexDestination {
            room_id: None,
            person_email: Some(destination.id.clone()),
            person_id: None,
        }),
        None => match infer_kind(id) {
            Some(kind) if kind == "room" => Ok(WebexDestination {
                room_id: Some(destination.id.clone()),
                person_email: None,
                person_id: None,
            }),
            _ => Ok(WebexDestination {
                room_id: None,
                person_email: Some(destination.id.clone()),
                person_id: None,
            }),
        },
        Some(kind) => Err(format!("unsupported destination kind: {kind}")),
    }
}

struct WebexDestination {
    room_id: Option<String>,
    person_email: Option<String>,
    person_id: Option<String>,
}

fn handle_reply(_input_json: &[u8]) -> Vec<u8> {
    let parsed: Value = match serde_json::from_slice(_input_json) {
        Ok(val) => val,
        Err(err) => {
            return json_bytes(&json!({"ok": false, "error": format!("invalid json: {err}")}));
        }
    };
    let cfg = match load_config(&parsed, None) {
        Ok(cfg) => cfg,
        Err(err) => return json_bytes(&json!({"ok": false, "error": err})),
    };

    let text = parsed
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if text.is_empty() {
        return json_bytes(&json!({"ok": false, "error": "text required"}));
    }
    let thread_id = parsed
        .get("reply_to_id")
        .or_else(|| parsed.get("thread_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if thread_id.is_empty() {
        return json_bytes(&json!({"ok": false, "error": "reply_to_id or thread_id required"}));
    }

    let token = match secrets_store::get(DEFAULT_TOKEN_KEY) {
        Ok(Some(bytes)) => String::from_utf8(bytes).unwrap_or_default(),
        _ => return json_bytes(&json!({"ok": false, "error": "missing access token"})),
    };
    if token.is_empty() {
        return json_bytes(&json!({"ok": false, "error": "access token empty"}));
    }
    let api_base = cfg
        .api_base_url
        .unwrap_or_else(|| DEFAULT_API_BASE.to_string());
    let url = format!("{}/messages", api_base);
    let payload = json!({
        "parentId": thread_id,
        "markdown": text,
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
            "error": format!("webex returned status {}", resp.status),
        }));
    }
    let body_bytes = resp.body.unwrap_or_default();
    let body_json: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    let msg_id = body_json
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("webex-reply")
        .to_string();
    let provider_message_id = format!("webex:{msg_id}");

    json_bytes(&json!({
        "ok": true,
        "status": "replied",
        "provider_type": PROVIDER_TYPE,
        "message_id": msg_id,
        "provider_message_id": provider_message_id,
        "response": body_json
    }))
}

fn parse_config_bytes(bytes: &[u8]) -> Result<ProviderConfig, String> {
    serde_json::from_slice::<ProviderConfig>(bytes).map_err(|e| format!("invalid config: {e}"))
}

fn parse_config_value(val: &Value) -> Result<ProviderConfig, String> {
    serde_json::from_value::<ProviderConfig>(val.clone())
        .map_err(|e| format!("invalid config: {e}"))
}

fn load_config(
    input: &Value,
    metadata: Option<&MessageMetadata>,
) -> Result<ProviderConfig, String> {
    if let Some(cfg) = input.get("config") {
        return parse_config_value(cfg);
    }
    let mut partial = serde_json::Map::new();
    for key in ["default_room_id", "api_base_url"] {
        if let Some(v) = input.get(key) {
            partial.insert(key.to_string(), v.clone());
        }
    }
    if let Some(meta) = metadata {
        for key in ["default_room_id", "api_base_url"] {
            if partial.get(key).is_none() {
                if let Some(value) = meta.get(key) {
                    partial.insert(key.to_string(), Value::String(value.clone()));
                }
            }
        }
    }
    if !partial.is_empty() {
        return parse_config_value(&Value::Object(partial));
    }

    Ok(ProviderConfig {
        default_room_id: None,
        api_base_url: None,
    })
}

fn json_bytes<T: serde::Serialize>(value: &T) -> Vec<u8> {
    serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec())
}

fn ingest_http(input_json: &[u8]) -> Vec<u8> {
    let request = match serde_json::from_slice::<HttpInV1>(input_json) {
        Ok(req) => req,
        Err(err) => return http_out_error(400, &format!("invalid http input: {err}")),
    };
    let body_bytes = match general_purpose::STANDARD.decode(&request.body_b64) {
        Ok(bytes) => bytes,
        Err(err) => return http_out_error(400, &format!("invalid body encoding: {err}")),
    };
    let body_val: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    let text = body_val
        .get("text")
        .or_else(|| body_val.get("markdown"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let (room_id, person_email, person_id) = extract_destination(&body_val);
    let envelope = build_envelope(
        "webex",
        text,
        room_id.clone(),
        person_email.clone(),
        person_id.clone(),
    );
    let normalized = json!({
        "ok": true,
        "event": body_val,
    });
    let normalized_bytes = serde_json::to_vec(&normalized).unwrap_or_else(|_| b"{}".to_vec());
    let out = HttpOutV1 {
        status: 200,
        headers: Vec::new(),
        body_b64: general_purpose::STANDARD.encode(&normalized_bytes),
        events: vec![envelope],
    };
    json_bytes(&out)
}

fn render_plan(input_json: &[u8]) -> Vec<u8> {
    let plan_in = match serde_json::from_slice::<RenderPlanInV1>(input_json) {
        Ok(value) => value,
        Err(err) => return render_plan_error(&format!("invalid render input: {err}")),
    };
    let summary = plan_in
        .message
        .text
        .clone()
        .filter(|text| !text.trim().is_empty())
        .unwrap_or_else(|| "webex message".to_string());
    let plan_obj = json!({
        "tier": "TierC",
        "summary_text": summary,
        "actions": [],
        "attachments": [],
        "warnings": [],
        "debug": plan_in.metadata,
    });
    let plan_json =
        serde_json::to_string(&plan_obj).unwrap_or_else(|_| "{\"tier\":\"TierC\"}".to_string());
    let plan_out = RenderPlanOutV1 { plan_json };
    json_bytes(&json!({"ok": true, "plan": plan_out}))
}

fn encode_op(input_json: &[u8]) -> Vec<u8> {
    let encode_in = match serde_json::from_slice::<EncodeInV1>(input_json) {
        Ok(value) => value,
        Err(err) => return encode_error(&format!("invalid encode input: {err}")),
    };
    let body_bytes = serde_json::to_vec(&encode_in.message)
        .unwrap_or_else(|_| serde_json::to_vec(&json!({})).unwrap());
    let mut metadata = HashMap::new();
    metadata.insert(
        "url".to_string(),
        Value::String(format!("{}/messages", DEFAULT_API_BASE)),
    );
    metadata.insert("method".to_string(), Value::String("POST".to_string()));
    let payload = ProviderPayloadV1 {
        content_type: "application/json".to_string(),
        body_b64: general_purpose::STANDARD.encode(&body_bytes),
        metadata,
    };
    json_bytes(&json!({"ok": true, "payload": payload}))
}

fn send_payload(input_json: &[u8]) -> Vec<u8> {
    let send_in = match serde_json::from_slice::<SendPayloadInV1>(input_json) {
        Ok(value) => value,
        Err(err) => {
            return send_payload_error(&format!("invalid send_payload input: {err}"), false);
        }
    };
    if send_in.provider_type != PROVIDER_TYPE {
        return send_payload_error("provider type mismatch", false);
    }
    let body_bytes = match general_purpose::STANDARD.decode(&send_in.payload.body_b64) {
        Ok(bytes) => bytes,
        Err(err) => return send_payload_error(&format!("payload decode failed: {err}"), false),
    };
    let payload: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    match invoke_handle_send(&payload) {
        Ok(_) => send_payload_success(),
        Err(err) => send_payload_error(&err, false),
    }
}

fn invoke_handle_send(payload: &Value) -> Result<(), String> {
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

fn extract_destination(payload: &Value) -> (Option<String>, Option<String>, Option<String>) {
    let direct_room = payload
        .get("roomId")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    if direct_room.is_some() {
        return (direct_room, None, None);
    }
    let direct_person_email = payload
        .get("toPersonEmail")
        .or_else(|| payload.get("personId"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let direct_person_id = payload
        .get("toPersonId")
        .or_else(|| payload.get("personId"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    if direct_person_email.is_some() || direct_person_id.is_some() {
        return (None, direct_person_email, direct_person_id);
    }
    let to = payload.get("to").and_then(|v| v.as_object());
    let kind = to
        .and_then(|o| o.get("kind"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let id = to
        .and_then(|o| o.get("id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let config_room = payload
        .get("config")
        .and_then(|c| c.get("default_room_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    if let Some(choice) = kind {
        match choice.as_str() {
            "room" => return (id.or(config_room), None, None),
            "personEmail" | "person_email" | "email" => return (None, id, None),
            "personId" | "person_id" => return (None, None, id),
            _ => return (None, id, None),
        }
    }
    (config_room, None, None)
}

fn build_envelope(
    channel_prefix: &str,
    text: String,
    room_id: Option<String>,
    person_email: Option<String>,
    person_id: Option<String>,
) -> ChannelMessageEnvelope {
    let env = EnvId::try_from("default").expect("env id");
    let tenant = TenantId::try_from("default").expect("tenant id");
    let mut metadata = MessageMetadata::new();
    metadata.insert("universal".to_string(), "true".to_string());
    if let Some(room) = &room_id {
        metadata.insert("room_id".to_string(), room.clone());
    }
    if let Some(person) = &person_email {
        metadata.insert("person_email".to_string(), person.clone());
    }
    if let Some(person) = &person_id {
        metadata.insert("person_id".to_string(), person.clone());
    }
    let channel = room_id
        .clone()
        .or(person_email.clone())
        .or(person_id.clone())
        .unwrap_or_else(|| channel_prefix.to_string());
    ChannelMessageEnvelope {
        id: format!("{channel_prefix}-{channel}"),
        tenant: TenantCtx::new(env.clone(), tenant.clone()),
        channel: channel.clone(),
        session_id: channel,
        reply_scope: None,
        from: person_id.clone().or(person_email.clone()).map(|id| Actor {
            id,
            kind: Some("user".to_string()),
        }),
        to: Vec::new(),
        correlation_id: None,
        text: Some(text),
        attachments: Vec::new(),
        metadata,
    }
}

fn http_out_error(status: u16, message: &str) -> Vec<u8> {
    let out = HttpOutV1 {
        status,
        headers: Vec::new(),
        body_b64: general_purpose::STANDARD.encode(message.as_bytes()),
        events: Vec::new(),
    };
    json_bytes(&out)
}

fn render_plan_error(message: &str) -> Vec<u8> {
    json_bytes(&json!({"ok": false, "error": message}))
}

fn encode_error(message: &str) -> Vec<u8> {
    json_bytes(&json!({"ok": false, "error": message}))
}

fn send_payload_error(message: &str, retryable: bool) -> Vec<u8> {
    let result = SendPayloadResultV1 {
        ok: false,
        message: Some(message.to_string()),
        retryable,
    };
    json_bytes(&result)
}

fn send_payload_success() -> Vec<u8> {
    let result = SendPayloadResultV1 {
        ok: true,
        message: None,
        retryable: false,
    };
    json_bytes(&result)
}

fn get_secret_string(key: &str) -> Result<String, String> {
    match secrets_store::get(key) {
        Ok(Some(bytes)) => String::from_utf8(bytes).map_err(|_| "secret not valid utf-8".into()),
        Ok(None) => Err(format!("missing secret: {key}")),
        Err(e) => Err(format!("secret store error: {e:?}")),
    }
}

fn destination_required_response() -> Value {
    json!({
        "message": "to field  required, either of kind person or room. person will be used if kind is not set.",
        "ok": false,
        "retryable": false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_accepts_defaults() {
        let cfg = br#"{"default_room_id":"room"}"#;
        let resp = Component::validate_config(cfg.to_vec());
        let json: Value = serde_json::from_slice(&resp).unwrap();
        assert_eq!(json.get("ok"), Some(&Value::Bool(true)));
    }

    #[test]
    fn load_config_defaults_to_token_key() {
        let input = json!({});
        let cfg = load_config(&input, None).unwrap();
        assert!(cfg.default_room_id.is_none());
    }

    #[test]
    fn parse_config_rejects_unknown() {
        let cfg = br#"{"default_room_id":"k","unexpected":true}"#;
        let err = parse_config_bytes(cfg).unwrap_err();
        assert!(err.contains("unknown field"));
    }

    #[test]
    fn destination_required_response_contains_message() {
        let resp = destination_required_response();
        assert_eq!(
            resp.get("message")
                .and_then(|v| v.as_str())
                .expect("message field"),
            "to field  required, either of kind person or room. person will be used if kind is not set."
        );
        assert_eq!(resp.get("ok"), Some(&Value::Bool(false)));
        assert_eq!(resp.get("retryable"), Some(&Value::Bool(false)));
    }
}
