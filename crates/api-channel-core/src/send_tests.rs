use crate::config::ApiConfig;
use crate::env::PostOutcome;
use crate::env::fake::FakeEnv;
use crate::event::{Event, EventType};
use crate::log;
use crate::send::run_send;
use crate::store::mem::MemStore;
use crate::webhook::{MAX_ATTEMPTS, SIGNATURE_HEADER, TIMESTAMP_HEADER, sign};
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{Value, json};
use sha2::Sha256;

const SECRET: &str = "whsec_test_secret";
const API_KEY: &str = "key_test_secret";

fn cfg(webhook: bool) -> ApiConfig {
    ApiConfig {
        api_key: Some(API_KEY.into()),
        webhook_url: webhook.then(|| "https://hooks.example.com/in".to_string()),
        webhook_secret: webhook.then(|| SECRET.to_string()),
    }
}

fn setup() -> (MemStore, FakeEnv) {
    let mut store = MemStore::default();
    log::ensure_conversation(&mut store, "c1").expect("create");
    (store, FakeEnv::default())
}

fn send(store: &mut MemStore, env: &mut FakeEnv, config: &ApiConfig, body: Value) -> Value {
    let out = run_send(store, env, config, body.to_string().as_bytes());
    serde_json::from_slice(&out).expect("json")
}

fn events(store: &mut MemStore) -> Vec<Event> {
    log::poll(store, "c1", 0).expect("poll").events
}

#[test]
fn send_appends_a_message_event() {
    let (mut store, mut env) = setup();
    let out = send(
        &mut store,
        &mut env,
        &cfg(false),
        json!({"conversation_id":"c1","text":"hi"}),
    );
    assert_eq!(out["ok"], true);
    assert_eq!(out["cursor"], 1);
    let evs = events(&mut store);
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].kind, EventType::Message);
    assert!(
        env.posts.is_empty(),
        "no webhook configured, nothing is posted"
    );
}

#[test]
fn send_can_append_each_event_type_with_a_code() {
    let (mut store, mut env) = setup();
    send(
        &mut store,
        &mut env,
        &cfg(false),
        json!({"conversation_id":"c1","type":"error","code":"flow_failed"}),
    );
    send(
        &mut store,
        &mut env,
        &cfg(false),
        json!({"conversation_id":"c1","type":"done"}),
    );
    let evs = events(&mut store);
    assert_eq!(evs[0].code.as_deref(), Some("flow_failed"));
    assert_eq!(evs[1].kind, EventType::Done);
}

#[test]
fn send_to_an_unknown_conversation_fails_cleanly() {
    let (mut store, mut env) = setup();
    let out = send(
        &mut store,
        &mut env,
        &cfg(false),
        json!({"conversation_id":"zzz","text":"hi"}),
    );
    assert_eq!(out["ok"], false);
    assert_eq!(out["error"]["code"], "conversation_not_found");
}

#[test]
fn the_webhook_body_is_signed_over_timestamp_dot_body() {
    let (mut store, mut env) = setup();
    send(
        &mut store,
        &mut env,
        &cfg(true),
        json!({"conversation_id":"c1","text":"hi"}),
    );
    assert_eq!(env.posts.len(), 1);
    let (url, headers, body) = &env.posts[0];
    assert_eq!(url, "https://hooks.example.com/in");
    let header = |name: &str| {
        headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
            .expect("header")
    };
    let ts = header(TIMESTAMP_HEADER);
    // Recompute independently of `sign` so the construction is pinned.
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(SECRET.as_bytes()).expect("key");
    mac.update(format!("{ts}.").as_bytes());
    mac.update(body);
    let expected: String = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(header(SIGNATURE_HEADER), format!("sha256={expected}"));
    assert_eq!(
        header(SIGNATURE_HEADER),
        sign(SECRET, ts.parse().expect("ts"), body)
    );
    // The body is the stored event.
    let posted: Event = serde_json::from_slice(body).expect("event");
    assert_eq!(posted.cursor, 1);
}

#[test]
fn a_5xx_is_retried_a_bounded_number_of_times_then_delivery_failed_is_recorded() {
    let (mut store, mut env) = setup();
    env.outcomes = vec![PostOutcome::Retryable; 10];
    let out = send(
        &mut store,
        &mut env,
        &cfg(true),
        json!({"conversation_id":"c1","text":"hi"}),
    );
    assert_eq!(out["ok"], true, "the turn is unaffected");
    assert_eq!(out["delivery_failed"], true);
    assert_eq!(env.posts.len() as u32, MAX_ATTEMPTS);
    assert_eq!(
        env.sleeps,
        vec![200, 400],
        "backoff doubles, none after the last try"
    );
    let evs = events(&mut store);
    assert_eq!(evs.last().map(|e| e.kind), Some(EventType::DeliveryFailed));
}

#[test]
fn a_4xx_is_not_retried() {
    let (mut store, mut env) = setup();
    env.outcomes = vec![PostOutcome::Rejected];
    let out = send(
        &mut store,
        &mut env,
        &cfg(true),
        json!({"conversation_id":"c1","text":"hi"}),
    );
    assert_eq!(out["delivery_failed"], true);
    assert_eq!(env.posts.len(), 1);
}

#[test]
fn a_transient_failure_then_success_delivers_without_a_failure_event() {
    let (mut store, mut env) = setup();
    env.outcomes = vec![PostOutcome::Retryable, PostOutcome::Delivered];
    let out = send(
        &mut store,
        &mut env,
        &cfg(true),
        json!({"conversation_id":"c1","text":"hi"}),
    );
    assert_eq!(out["delivery_failed"], false);
    assert_eq!(events(&mut store).len(), 1);
}

#[test]
fn a_non_https_webhook_is_never_called() {
    let (mut store, mut env) = setup();
    let mut config = cfg(true);
    config.webhook_url = Some("http://169.254.169.254/latest".into());
    let out = send(
        &mut store,
        &mut env,
        &config,
        json!({"conversation_id":"c1","text":"hi"}),
    );
    assert!(env.posts.is_empty());
    assert_eq!(out["delivery_failed"], true);
}

#[test]
fn secrets_never_appear_in_output_events_or_debug() {
    let (mut store, mut env) = setup();
    let config = cfg(true);
    let out = run_send(
        &mut store,
        &mut env,
        &config,
        br#"{"conversation_id":"c1","text":"hi"}"#,
    );
    let mut haystack = String::from_utf8(out).expect("utf8");
    haystack.push_str(&serde_json::to_string(&events(&mut store)).expect("json"));
    haystack.push_str(&format!("{config:?}"));
    for (_, headers, body) in &env.posts {
        haystack.push_str(&String::from_utf8_lossy(body));
        for (k, _) in headers {
            haystack.push_str(k);
        }
    }
    assert!(!haystack.contains(SECRET));
    assert!(!haystack.contains(API_KEY));
}
