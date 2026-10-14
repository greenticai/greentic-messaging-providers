use crate::config::ApiConfig;
use crate::env::fake::FakeEnv;
use crate::event::{Item, NewEvent};
use crate::http::HttpRequest;
use crate::ingest::ingest_http;
use crate::log::{self, RETAINED_EVENTS};
use crate::store::mem::MemStore;
use serde_json::Value;

fn config() -> ApiConfig {
    ApiConfig {
        api_key: Some("key_test_secret".into()),
        ..ApiConfig::default()
    }
}

fn get(auth: bool, query: &str) -> HttpRequest {
    let mut headers = Vec::new();
    if auth {
        headers.push((
            "Authorization".to_string(),
            "Bearer key_test_secret".to_string(),
        ));
    }
    HttpRequest {
        method: "GET".into(),
        path: "/v1/messaging/api/acme/messages".into(),
        query: query.into(),
        headers,
        body: Vec::new(),
    }
}

fn seeded(n: u64) -> (MemStore, FakeEnv) {
    let mut store = MemStore::default();
    let env = FakeEnv::default();
    log::ensure_conversation(&mut store, "c1").expect("create");
    for i in 0..n {
        let item = Item::Text {
            text: i.to_string(),
        };
        log::append(&mut store, &env, "c1", NewEvent::message(vec![item])).expect("append");
    }
    (store, env)
}

fn run(store: &mut MemStore, env: &mut FakeEnv, req: &HttpRequest) -> (u16, Value) {
    let out = ingest_http(store, env, &config(), req);
    (
        out.response.status,
        serde_json::from_slice(&out.response.body).expect("json"),
    )
}

#[test]
fn get_without_a_bearer_is_401() {
    let (mut store, mut env) = seeded(1);
    assert_eq!(
        run(&mut store, &mut env, &get(false, "conversation_id=c1")).0,
        401
    );
}

#[test]
fn get_returns_events_after_the_cursor_in_the_poll_shape() {
    let (mut store, mut env) = seeded(3);
    let (status, body) = run(
        &mut store,
        &mut env,
        &get(true, "conversation_id=c1&after=2"),
    );
    assert_eq!(status, 200);
    let cursors: Vec<u64> = body["events"]
        .as_array()
        .expect("events")
        .iter()
        .map(|e| e["cursor"].as_u64().expect("cursor"))
        .collect();
    assert_eq!(cursors, vec![2, 3]);
    assert_eq!(body["next_cursor"], 4);
}

#[test]
fn get_without_after_starts_from_the_beginning() {
    let (mut store, mut env) = seeded(2);
    let (_, body) = run(&mut store, &mut env, &get(true, "conversation_id=c1"));
    assert_eq!(body["events"].as_array().map(Vec::len), Some(2));
}

#[test]
fn get_for_an_unknown_conversation_is_404() {
    let (mut store, mut env) = seeded(1);
    let (status, body) = run(&mut store, &mut env, &get(true, "conversation_id=zzz"));
    assert_eq!(status, 404);
    assert_eq!(body["error"]["code"], "conversation_not_found");
}

#[test]
fn get_with_an_expired_cursor_is_410_naming_the_oldest() {
    let (mut store, mut env) = seeded(RETAINED_EVENTS + 5);
    let (status, body) = run(
        &mut store,
        &mut env,
        &get(true, "conversation_id=c1&after=1"),
    );
    assert_eq!(status, 410);
    assert_eq!(body["error"]["code"], "cursor_expired");
    assert_eq!(body["error"]["oldest_cursor"], 6);
}

#[test]
fn get_validates_its_parameters() {
    let (mut store, mut env) = seeded(1);
    assert_eq!(run(&mut store, &mut env, &get(true, "")).0, 400);
    assert_eq!(
        run(
            &mut store,
            &mut env,
            &get(true, "conversation_id=c1&after=abc")
        )
        .0,
        400
    );
    assert_eq!(
        run(&mut store, &mut env, &get(true, "conversation_id=bad%20id")).0,
        400
    );
}
