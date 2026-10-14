use crate::env::fake::FakeEnv;
use crate::event::{EventType, Item, NewEvent};
use crate::log::{self, LogError, PollOutcome, RETAINED_EVENTS};
use crate::store::StateStore;
use crate::store::mem::MemStore;

fn text(t: &str) -> NewEvent {
    NewEvent::message(vec![Item::Text { text: t.into() }])
}

fn setup() -> (MemStore, FakeEnv) {
    let mut store = MemStore::default();
    log::ensure_conversation(&mut store, "c1").expect("create");
    (store, FakeEnv::default())
}

#[test]
fn cursors_are_monotonic_and_start_at_one() {
    let (mut store, env) = setup();
    let a = log::append(&mut store, &env, "c1", text("a")).expect("append");
    let b = log::append(&mut store, &env, "c1", text("b")).expect("append");
    assert_eq!((a.cursor, b.cursor), (1, 2));
}

#[test]
fn poll_returns_events_from_since_in_ascending_order() {
    let (mut store, env) = setup();
    for t in ["a", "b", "c"] {
        log::append(&mut store, &env, "c1", text(t)).expect("append");
    }
    let PollOutcome {
        events,
        next_cursor,
    } = log::poll(&mut store, "c1", 2).expect("poll");
    let cursors: Vec<u64> = events.iter().map(|e| e.cursor).collect();
    assert_eq!(cursors, vec![2, 3]);
    assert_eq!(next_cursor, 4);
}

#[test]
fn since_zero_means_from_the_beginning() {
    let (mut store, env) = setup();
    log::append(&mut store, &env, "c1", text("a")).expect("append");
    let out = log::poll(&mut store, "c1", 0).expect("poll");
    assert_eq!(out.events.len(), 1);
}

#[test]
fn an_empty_poll_hands_the_cursor_back() {
    let (mut store, env) = setup();
    log::append(&mut store, &env, "c1", text("a")).expect("append");
    let out = log::poll(&mut store, "c1", 2).expect("poll");
    assert!(out.events.is_empty());
    assert_eq!(out.next_cursor, 2);
}

#[test]
fn an_unknown_conversation_is_not_found() {
    let (mut store, _env) = setup();
    assert_eq!(
        log::poll(&mut store, "nope", 1),
        Err(LogError::ConversationNotFound)
    );
}

#[test]
fn a_cursor_older_than_the_retained_log_is_expired_with_the_oldest() {
    let (mut store, env) = setup();
    let total = RETAINED_EVENTS + 5;
    for i in 0..total {
        log::append(&mut store, &env, "c1", text(&i.to_string())).expect("append");
    }
    let oldest = total - RETAINED_EVENTS + 1;
    assert_eq!(
        log::poll(&mut store, "c1", 1),
        Err(LogError::CursorExpired {
            oldest_cursor: oldest
        })
    );
    let ok = log::poll(&mut store, "c1", oldest).expect("poll at oldest");
    assert_eq!(ok.events.first().map(|e| e.cursor), Some(oldest));
    // since == 0 asks for "everything you still have", never an error.
    assert!(log::poll(&mut store, "c1", 0).is_ok());
}

#[test]
fn two_appenders_with_a_stale_head_never_share_a_cursor() {
    let (mut store, env) = setup();
    log::append(&mut store, &env, "c1", text("a")).expect("append");
    // Simulate a lagging head hint written by a slow concurrent appender.
    store.write("api/v1/c1/head", b"0").expect("write");
    let b = log::append(&mut store, &env, "c1", text("b")).expect("append");
    assert_eq!(b.cursor, 2);
}

#[test]
fn without_atomic_create_the_log_refuses_to_append() {
    let (mut store, env) = setup();
    store.no_atomic_create = true;
    assert_eq!(
        log::append(&mut store, &env, "c1", text("a")),
        Err(LogError::Unsupported)
    );
}

#[test]
fn error_events_carry_a_code_and_no_provider_text() {
    let (mut store, env) = setup();
    let e = log::append(&mut store, &env, "c1", NewEvent::error("flow_failed")).expect("append");
    assert_eq!(e.kind, EventType::Error);
    assert_eq!(e.code.as_deref(), Some("flow_failed"));
}

#[test]
fn the_poll_request_and_response_match_the_shared_fixtures() {
    let req = std::fs::read_to_string(format!(
        "{}/../../docs/fixtures/api-channel/poll_events_request.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("request fixture");
    let resp = std::fs::read_to_string(format!(
        "{}/../../docs/fixtures/api-channel/poll_events_response.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("response fixture");
    let mut store = MemStore::default();
    let mut env = FakeEnv::default();
    log::ensure_conversation(&mut store, "c1").expect("create");
    // The fixture's message is cursor 7, so burn six cursors first.
    for _ in 0..6 {
        log::append(&mut store, &env, "c1", NewEvent::of(EventType::Done)).expect("append");
    }
    log::append(&mut store, &env, "c1", text("hi")).expect("append");
    let out = log::run_poll_events(&mut store, req.as_bytes());
    assert_eq!(String::from_utf8(out).expect("utf8"), resp);
    let _ = &mut env;
}
