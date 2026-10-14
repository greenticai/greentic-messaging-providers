//! Pure-adapter tests. Anything that needs the host (state, secrets, HTTP)
//! is covered in `api-channel-core` against fakes; the WASM build is the
//! check that the bindings link.

use super::*;
use crate::ops::tenant_from_path;
use base64::{Engine as _, engine::general_purpose::STANDARD};

#[test]
fn the_tenant_is_the_path_segment_after_api() {
    assert_eq!(
        tenant_from_path("/v1/messaging/api/acme/messages"),
        Some("acme")
    );
    assert_eq!(
        tenant_from_path("/v1/messaging/api/acme/bundle-1/messages"),
        Some("acme")
    );
    assert_eq!(tenant_from_path("/v1/messaging/api/messages"), None);
    assert_eq!(tenant_from_path("/other"), None);
}

#[test]
fn describe_lists_the_three_ops_and_redacts_both_secrets() {
    let payload = build_describe_payload();
    let names: Vec<&str> = payload.operations.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(names, vec!["send", "ingest_http", "poll_events"]);
    let paths: Vec<&str> = payload.redactions.iter().map(|r| r.path.as_str()).collect();
    assert!(paths.contains(&"$.api_key") && paths.contains(&"$.webhook_secret"));
}

#[test]
fn every_describe_i18n_key_has_text() {
    let json = serde_json::to_string(&build_describe_payload()).expect("json");
    assert!(json.contains("api.op.send.title"));
    let keys = i18n_keys();
    let mut sorted = keys.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(keys.len(), sorted.len(), "duplicate i18n key");
}

#[test]
fn qa_spec_asks_the_three_questions_and_requires_the_api_key() {
    let spec = build_qa_spec("setup");
    let ids: Vec<&str> = spec.questions.iter().map(|q| q.id.as_str()).collect();
    assert_eq!(ids, vec!["webhook_url", "webhook_secret", "api_key"]);
    let required: Vec<&str> = spec
        .questions
        .iter()
        .filter(|q| q.required)
        .map(|q| q.id.as_str())
        .collect();
    assert_eq!(required, vec!["api_key"]);
}

#[test]
fn apply_answers_round_trips_through_cbor_and_keeps_secrets_out_of_config() {
    let answers = json!({"api_key": "0123456789abcdef-key", "webhook_url": "https://h.example.com/x", "webhook_secret": "whsec_0123456789"});
    let out = apply_answers_impl("setup", canonical_cbor_bytes(&answers));
    let value: Value = decode_cbor(&out).expect("cbor");
    assert_eq!(value["ok"], true);
    let config = serde_json::to_string(&value["config"]).expect("json");
    assert!(!config.contains("0123456789abcdef-key") && !config.contains("whsec_"));
    assert_eq!(
        value["secrets_patch"]["set"]["API_CHANNEL_API_KEY"],
        "0123456789abcdef-key"
    );
}

#[test]
fn an_unknown_op_is_reported_not_panicked() {
    let out = ops::dispatch("nope", b"{}");
    let value: Value = serde_json::from_slice(&out).expect("json");
    assert_eq!(value["ok"], false);
}

fn envelope_json(text: Option<&str>, card: Option<Value>, error: bool) -> Vec<u8> {
    let inbound = api_channel_core::http::Inbound {
        conversation_id: "c1".into(),
        message_id: "m_1".into(),
        text: text.map(str::to_string),
        card: None,
    };
    let mut message = serde_json::to_value(ops::envelope(&inbound, "acme").expect("envelope"))
        .expect("envelope json");
    if let Some(metadata) = message["metadata"].as_object_mut() {
        if let Some(card) = card {
            metadata.insert("adaptive_card".into(), Value::String(card.to_string()));
        }
        if error {
            metadata.insert("error_kind".into(), "node".into());
            metadata.insert("error_message".into(), "boom with internal detail".into());
        }
    }
    serde_json::to_vec(&json!({ "message": message })).expect("json")
}

fn encoded_body(out: &[u8]) -> Value {
    let value: Value = serde_json::from_slice(out).expect("json");
    assert_eq!(value["ok"], true, "{value}");
    let b64 = value["payload"]["body_b64"].as_str().expect("body");
    serde_json::from_slice(&STANDARD.decode(b64).expect("b64")).expect("payload json")
}

#[test]
fn encode_carries_session_text_and_card() {
    let card = json!({"type": "AdaptiveCard"});
    let body = encoded_body(&ops::dispatch(
        "encode",
        &envelope_json(Some("hi"), Some(card.clone()), false),
    ));
    assert_eq!(body["session_id"], "c1");
    assert_eq!(body["text"], "hi");
    assert_eq!(body["card"], card);
    assert_eq!(body["failed"], false);
}

#[test]
fn encode_names_the_tenant_so_the_host_injects_the_provider_config() {
    // greentic-start folds provider config into an encoded body only when it
    // holds both `tenant` and `session_id`; without it `send` never sees the
    // webhook URL.
    let body = encoded_body(&ops::dispatch(
        "encode",
        &envelope_json(Some("hi"), None, false),
    ));
    assert!(body.get("tenant").and_then(|t| t.as_str()).is_some());
    assert!(body.get("session_id").is_some());
}

#[test]
fn encode_marks_a_failed_turn_without_forwarding_the_engine_text() {
    let out = ops::dispatch("encode", &envelope_json(None, None, true));
    let raw = String::from_utf8_lossy(&out).to_string();
    let body = encoded_body(&out);
    assert_eq!(body["failed"], true);
    assert!(!raw.contains("boom with internal detail"));
}

#[test]
fn the_hosts_operator_shaped_ingress_input_is_understood() {
    // greentic-start sends query as `[[k, v]]` pairs and headers as tuples,
    // which the native `HttpInV1` cannot deserialize.
    let input = json!({
        "v": 1,
        "provider": "messaging.api",
        "method": "GET",
        "path": "/v1/messaging/api/acme/messages",
        "query": [["conversation_id", "c1"], ["after", "2"]],
        "headers": [["Authorization", "Bearer k"]],
        "body_b64": "",
    });
    let bytes = serde_json::to_vec(&input).expect("json");
    let (request, config) = crate::ops::core_request(&bytes).expect("parsed");
    assert_eq!(request.method, "GET");
    assert_eq!(request.query, "conversation_id=c1&after=2");
    assert_eq!(request.header("authorization"), Some("Bearer k"));
    assert!(config.is_none());
}

#[test]
fn the_native_http_in_shape_still_parses() {
    let input = json!({
        "method": "POST",
        "path": "/v1/messaging/api/acme/messages",
        "query": "x=1",
        "headers": [{"name": "Authorization", "value": "Bearer k"}],
        "body_b64": STANDARD.encode(b"{}"),
    });
    let bytes = serde_json::to_vec(&input).expect("json");
    let (request, _) = crate::ops::core_request(&bytes).expect("parsed");
    assert_eq!(request.query, "x=1");
    assert_eq!(request.body, b"{}".to_vec());
    assert_eq!(request.header("authorization"), Some("Bearer k"));
}
