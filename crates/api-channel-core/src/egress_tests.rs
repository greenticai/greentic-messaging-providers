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

#[test]
fn a_sent_event_announces_itself_so_the_host_can_wake_streams() {
    use crate::egress::notify_metadata;
    let mut payload = encode_payload("c1", Some("hi"), None, false, false);
    payload["tenant"] = json!("acme");
    let sent = json!({"ok": true, "cursor": 7, "delivery_failed": false});
    let meta = notify_metadata(&payload, &sent).expect("metadata");
    assert_eq!(meta["tenant"], "acme");
    assert_eq!(meta["conversation_id"], "c1");
    assert_eq!(meta["watermark_bumped"], 7);

    // The delivery_failed marker took the next cursor.
    let sent = json!({"ok": true, "cursor": 7, "delivery_failed": true});
    assert_eq!(
        notify_metadata(&payload, &sent).expect("m")["watermark_bumped"],
        8
    );

    // Nothing to announce when the send failed or the tenant is unknown.
    assert!(notify_metadata(&payload, &json!({"ok": false})).is_none());
    let no_tenant = encode_payload("c1", Some("hi"), None, false, false);
    assert!(notify_metadata(&no_tenant, &json!({"ok": true, "cursor": 1})).is_none());
}
