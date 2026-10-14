//! The shared fixtures are asserted byte for byte: a serialized value must
//! equal the file, and the file must parse back to the same value.

use crate::event::{Event, EventType, Item};
use serde::Serialize;
use serde_json::json;

fn fixture(name: &str) -> String {
    let path = format!(
        "{}/../../docs/fixtures/api-channel/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("read {path}: {err}"))
}

fn assert_bytes<T: Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug>(
    name: &str,
    value: &T,
) {
    let text = fixture(name);
    assert_eq!(
        serde_json::to_string(value).expect("serialize"),
        text,
        "{name} drifted"
    );
    let parsed: T = serde_json::from_str(&text).expect("parse fixture");
    assert_eq!(&parsed, value, "{name} does not parse back");
}

fn ev(cursor: u64, kind: EventType, code: Option<&str>, items: Vec<Item>, ts: &str) -> Event {
    Event {
        cursor,
        kind,
        code: code.map(str::to_string),
        items,
        ts: ts.to_string(),
    }
}

#[test]
fn message_fixture() {
    assert_bytes(
        "message.json",
        &ev(
            7,
            EventType::Message,
            None,
            vec![Item::Text { text: "hi".into() }],
            "2026-10-14T10:00:00Z",
        ),
    );
}

#[test]
fn input_required_fixture() {
    assert_bytes(
        "input_required.json",
        &ev(
            8,
            EventType::InputRequired,
            None,
            vec![Item::Card {
                card: json!({"type":"AdaptiveCard","version":"1.5","body":[]}),
                fallback: "Please pick an option".into(),
            }],
            "2026-10-14T10:00:01Z",
        ),
    );
}

#[test]
fn error_fixture() {
    assert_bytes(
        "error.json",
        &ev(
            9,
            EventType::Error,
            Some("flow_failed"),
            vec![],
            "2026-10-14T10:00:02Z",
        ),
    );
}

#[test]
fn done_fixture() {
    assert_bytes(
        "done.json",
        &ev(10, EventType::Done, None, vec![], "2026-10-14T10:00:03Z"),
    );
}

#[test]
fn delivery_failed_fixture() {
    assert_bytes(
        "delivery_failed.json",
        &ev(
            11,
            EventType::DeliveryFailed,
            None,
            vec![],
            "2026-10-14T10:00:04Z",
        ),
    );
}
