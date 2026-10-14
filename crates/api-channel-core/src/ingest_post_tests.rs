use crate::config::ApiConfig;
use crate::env::fake::FakeEnv;
use crate::http::HttpRequest;
use crate::ingest::{MAX_BODY_BYTES, ingest_http};
use crate::store::mem::MemStore;
use serde_json::{Value, json};

const KEY: &str = "key_test_secret";

fn config() -> ApiConfig {
    ApiConfig {
        api_key: Some(KEY.into()),
        ..ApiConfig::default()
    }
}

fn post(auth: Option<&str>, body: Value) -> HttpRequest {
    let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
    if let Some(auth) = auth {
        headers.push(("authorization".to_string(), auth.to_string()));
    }
    HttpRequest {
        method: "POST".into(),
        path: "/v1/messaging/api/acme/messages".into(),
        query: String::new(),
        headers,
        body: body.to_string().into_bytes(),
    }
}

fn bearer() -> Option<&'static str> {
    Some("Bearer key_test_secret")
}

fn run(req: &HttpRequest) -> (crate::http::IngestResult, MemStore) {
    let mut store = MemStore::default();
    let mut env = FakeEnv::default();
    let out = ingest_http(&mut store, &mut env, &config(), req);
    (out, store)
}

fn json_body(out: &crate::http::IngestResult) -> Value {
    serde_json::from_slice(&out.response.body).expect("json body")
}

#[test]
fn a_missing_bearer_is_401_before_any_state_or_flow() {
    let (out, store) = run(&post(None, json!({"client_message_id":"m1","text":"hi"})));
    assert_eq!(out.response.status, 401);
    assert!(out.inbound.is_none());
    assert!(
        store.map.is_empty(),
        "an unauthenticated call touches no state"
    );
}

#[test]
fn a_wrong_bearer_is_401_and_the_body_names_no_detail() {
    let (out, _) = run(&post(
        Some("Bearer nope"),
        json!({"client_message_id":"m1","text":"hi"}),
    ));
    assert_eq!(out.response.status, 401);
    assert_eq!(json_body(&out), json!({"error": {"code": "unauthorized"}}));
}

#[test]
fn a_prefix_of_the_key_is_not_accepted() {
    let (out, _) = run(&post(
        Some("Bearer key_test"),
        json!({"client_message_id":"m1","text":"hi"}),
    ));
    assert_eq!(out.response.status, 401);
}

#[test]
fn an_unset_key_refuses_everything() {
    let mut store = MemStore::default();
    let mut env = FakeEnv::default();
    let req = post(bearer(), json!({"client_message_id":"m1","text":"hi"}));
    let out = ingest_http(&mut store, &mut env, &ApiConfig::default(), &req);
    assert_eq!(out.response.status, 401);
}

#[test]
fn the_scheme_and_header_name_are_case_insensitive() {
    let (out, _) = run(&post(
        Some("bearer key_test_secret"),
        json!({"client_message_id":"m1","text":"hi"}),
    ));
    assert_eq!(out.response.status, 202);
}

#[test]
fn a_valid_post_is_202_with_ids_and_hands_the_message_to_the_flow() {
    let (out, _) = run(&post(
        bearer(),
        json!({"conversation_id":"c1","client_message_id":"m1","text":"hi"}),
    ));
    assert_eq!(out.response.status, 202);
    let body = json_body(&out);
    assert_eq!(body["conversation_id"], "c1");
    let inbound = out.inbound.expect("inbound");
    assert_eq!(inbound.text.as_deref(), Some("hi"));
    assert_eq!(body["message_id"], inbound.message_id);
}

#[test]
fn a_missing_conversation_id_is_server_minted() {
    let (out, _) = run(&post(
        bearer(),
        json!({"client_message_id":"m1","text":"hi"}),
    ));
    assert_eq!(json_body(&out)["conversation_id"], "c_rnd0001");
}

#[test]
fn a_retry_of_the_same_client_message_id_is_one_turn_and_the_same_message_id() {
    let mut store = MemStore::default();
    let mut env = FakeEnv::default();
    let req = post(
        bearer(),
        json!({"conversation_id":"c1","client_message_id":"m1","text":"hi"}),
    );
    let first = ingest_http(&mut store, &mut env, &config(), &req);
    let second = ingest_http(&mut store, &mut env, &config(), &req);
    assert_eq!(second.response.status, 202);
    assert_eq!(first.response.body, second.response.body);
    assert!(first.inbound.is_some());
    assert!(
        second.inbound.is_none(),
        "the retry must not start a second turn"
    );
}

#[test]
fn the_same_client_message_id_in_another_conversation_is_a_different_message() {
    let mut store = MemStore::default();
    let mut env = FakeEnv::default();
    let a = ingest_http(
        &mut store,
        &mut env,
        &config(),
        &post(
            bearer(),
            json!({"conversation_id":"c1","client_message_id":"m1","text":"a"}),
        ),
    );
    let b = ingest_http(
        &mut store,
        &mut env,
        &config(),
        &post(
            bearer(),
            json!({"conversation_id":"c2","client_message_id":"m1","text":"a"}),
        ),
    );
    assert!(a.inbound.is_some() && b.inbound.is_some());
}

#[test]
fn a_card_can_be_sent() {
    let (out, _) = run(&post(
        bearer(),
        json!({"conversation_id":"c1","client_message_id":"m1","card":{"type":"AdaptiveCard"}}),
    ));
    assert_eq!(out.response.status, 202);
    assert!(out.inbound.expect("inbound").card.is_some());
}

#[test]
fn validation_failures_are_400_with_a_code() {
    let cases = [
        (json!({"text":"hi"}), "client_message_id_required"),
        (json!({"client_message_id":"m1"}), "text_or_card_required"),
        (
            json!({"client_message_id":"m1","text":"hi","conversation_id":"bad id!"}),
            "invalid_conversation_id",
        ),
    ];
    for (body, code) in cases {
        let (out, _) = run(&post(bearer(), body));
        assert_eq!(out.response.status, 400, "{code}");
        assert_eq!(json_body(&out)["error"]["code"], code);
        assert!(out.inbound.is_none());
    }
    let mut req = post(bearer(), json!({}));
    req.body = b"not json".to_vec();
    assert_eq!(run(&req).0.response.status, 400);
}

#[test]
fn an_oversized_body_is_413() {
    let mut req = post(bearer(), json!({}));
    req.body = vec![b' '; MAX_BODY_BYTES + 1];
    assert_eq!(run(&req).0.response.status, 413);
}

#[test]
fn other_paths_and_methods_are_refused() {
    let mut req = post(bearer(), json!({"client_message_id":"m1","text":"hi"}));
    req.path = "/v1/messaging/api/acme/other".into();
    assert_eq!(run(&req).0.response.status, 404);
    let mut req = post(bearer(), json!({}));
    req.method = "DELETE".into();
    assert_eq!(run(&req).0.response.status, 405);
}
