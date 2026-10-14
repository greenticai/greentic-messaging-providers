use crate::config::ApiConfig;
use crate::egress::{encode_payload, send_input};
use crate::env::fake::FakeEnv;
use crate::event::{EventType, Item};
use crate::log;
use crate::send::run_send;
use crate::store::mem::MemStore;
use serde_json::json;

fn deliver(payload: serde_json::Value) -> crate::event::Event {
    let mut store = MemStore::default();
    let mut env = FakeEnv::default();
    log::ensure_conversation(&mut store, "c1").expect("create");
    let input = send_input(&payload).expect("send input");
    run_send(
        &mut store,
        &mut env,
        &ApiConfig::default(),
        input.to_string().as_bytes(),
    );
    log::poll(&mut store, "c1", 0)
        .expect("poll")
        .events
        .remove(0)
}

#[test]
fn a_text_reply_becomes_a_message_event() {
    let event = deliver(encode_payload("c1", Some("hello"), None, false, false));
    assert_eq!(event.kind, EventType::Message);
    assert_eq!(
        event.items,
        vec![Item::Text {
            text: "hello".into()
        }]
    );
}

#[test]
fn a_card_reply_carries_the_card_and_its_text_fallback() {
    let card = json!({"type": "AdaptiveCard"});
    let event = deliver(encode_payload(
        "c1",
        Some("pick one"),
        Some(card.clone()),
        false,
        true,
    ));
    assert_eq!(event.kind, EventType::InputRequired);
    assert_eq!(
        event.items,
        vec![Item::Card {
            card,
            fallback: "pick one".into()
        }]
    );
}

#[test]
fn a_failed_turn_is_an_error_event_with_a_generic_code_and_no_text() {
    let event = deliver(encode_payload(
        "c1",
        Some("secret provider stack trace"),
        None,
        true,
        false,
    ));
    assert_eq!(event.kind, EventType::Error);
    assert_eq!(event.code.as_deref(), Some("flow_failed"));
    assert!(event.items.is_empty());
}

#[test]
fn an_empty_reply_or_a_missing_session_is_refused() {
    assert!(send_input(&encode_payload("c1", None, None, false, false)).is_err());
    assert!(send_input(&json!({"text": "x"})).is_err());
}

#[test]
fn the_hosts_injected_config_reaches_the_send_input() {
    // greentic-start folds the deployment's config overrides into the encoded
    // payload body; without forwarding them the webhook URL never arrives.
    let mut payload = encode_payload("c1", Some("hi"), None, false, false);
    payload["config"] = json!({"webhook_url": "https://hook.example.com/x"});
    let input = send_input(&payload).expect("send input");
    assert_eq!(input["config"]["webhook_url"], "https://hook.example.com/x");

    let failed = send_input(&json!({
        "session_id": "c1", "failed": true,
        "config": {"webhook_url": "https://hook.example.com/x"}
    }))
    .expect("send input");
    assert_eq!(
        failed["config"]["webhook_url"],
        "https://hook.example.com/x"
    );

    let plain = send_input(&encode_payload("c1", Some("hi"), None, false, false)).expect("input");
    assert!(plain.get("config").is_none());
}
