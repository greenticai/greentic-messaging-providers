//! `GET .../conversations/{id}/stream` is an auth-only probe: greentic-start
//! serves the SSE stream itself and cannot verify the bearer, so it asks this
//! op and reads the status.

use crate::config::ApiConfig;
use crate::env::fake::FakeEnv;
use crate::http::HttpRequest;
use crate::ingest::ingest_http;
use crate::store::mem::MemStore;

fn config() -> ApiConfig {
    ApiConfig {
        api_key: Some("key_test_secret".into()),
        ..ApiConfig::default()
    }
}

fn probe(path: &str, authorization: Option<&str>) -> HttpRequest {
    HttpRequest {
        method: "GET".into(),
        path: path.into(),
        query: String::new(),
        headers: authorization
            .map(|v| vec![("Authorization".to_string(), v.to_string())])
            .unwrap_or_default(),
        body: Vec::new(),
    }
}

const STREAM: &str = "/v1/messaging/api/acme/conversations/c1/stream";
const BUNDLE_STREAM: &str = "/v1/messaging/api/acme/bundle-1/conversations/c1/stream";

fn status(req: &HttpRequest) -> (u16, MemStore) {
    let mut store = MemStore::default();
    let mut env = FakeEnv::default();
    let out = ingest_http(&mut store, &mut env, &config(), req);
    assert!(out.inbound.is_none(), "a probe never starts a turn");
    (out.response.status, store)
}

#[test]
fn a_missing_bearer_is_401() {
    assert_eq!(status(&probe(STREAM, None)).0, 401);
    assert_eq!(status(&probe(BUNDLE_STREAM, None)).0, 401);
}

#[test]
fn a_wrong_bearer_is_401_with_no_detail() {
    let mut store = MemStore::default();
    let mut env = FakeEnv::default();
    let out = ingest_http(
        &mut store,
        &mut env,
        &config(),
        &probe(STREAM, Some("Bearer nope")),
    );
    assert_eq!(out.response.status, 401);
    let body = String::from_utf8(out.response.body).expect("utf8");
    assert!(!body.contains("key_test_secret") && !body.contains("nope"));
}

#[test]
fn a_good_bearer_is_2xx_and_touches_no_state() {
    for path in [STREAM, BUNDLE_STREAM] {
        let (code, store) = status(&probe(path, Some("Bearer key_test_secret")));
        assert!((200..300).contains(&code), "{path} -> {code}");
        assert!(store.map.is_empty(), "the probe must not write");
    }
}

#[test]
fn an_unconfigured_key_refuses_the_probe() {
    let mut store = MemStore::default();
    let mut env = FakeEnv::default();
    let out = ingest_http(
        &mut store,
        &mut env,
        &ApiConfig::default(),
        &probe(STREAM, Some("Bearer anything")),
    );
    assert_eq!(out.response.status, 401);
}

#[test]
fn only_get_probes_and_only_the_stream_shape() {
    let mut post = probe(STREAM, Some("Bearer key_test_secret"));
    post.method = "POST".into();
    assert_eq!(status(&post).0, 405);
    let odd = probe(
        "/v1/messaging/api/acme/conversations/c1/other",
        Some("Bearer key_test_secret"),
    );
    assert_eq!(status(&odd).0, 404);
}
