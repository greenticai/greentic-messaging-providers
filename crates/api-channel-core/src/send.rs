//! The `send` provider op: append a reply to the log, then push the webhook.

use crate::config::ApiConfig;
use crate::env::Env;
use crate::event::{EventType, Item, NewEvent};
use crate::log::{self, LogError};
use crate::store::StateStore;
use crate::webhook::{self, Delivery};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
struct SendRequest {
    conversation_id: String,
    #[serde(default, rename = "type")]
    kind: Option<EventType>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    items: Vec<Item>,
}

/// JSON in, JSON out. Never fails because of the webhook: a receiver that is
/// down becomes a `delivery_failed` event, the turn is unaffected.
pub fn run_send<E: Env + ?Sized>(
    store: &mut dyn StateStore,
    env: &mut E,
    config: &ApiConfig,
    input: &[u8],
) -> Vec<u8> {
    let value = send(store, env, config, input);
    serde_json::to_vec(&value).unwrap_or_default()
}

fn send<E: Env + ?Sized>(
    store: &mut dyn StateStore,
    env: &mut E,
    config: &ApiConfig,
    input: &[u8],
) -> Value {
    let Ok(req) = serde_json::from_slice::<SendRequest>(input) else {
        return json!({"ok": false, "error": {"code": "invalid_request"}});
    };
    let mut items = req.items;
    if let Some(text) = req.text.filter(|t| !t.is_empty()) {
        items.insert(0, Item::Text { text });
    }
    let kind = req.kind.unwrap_or(EventType::Message);
    let new = NewEvent {
        kind,
        code: req.code,
        items,
    };
    let event = match log::append(store, env, &req.conversation_id, new) {
        Ok(event) => event,
        Err(err) => return failure(&err),
    };
    let mut delivery_failed = false;
    if webhook::deliver(env, config, &event) == Delivery::Failed {
        delivery_failed = true;
        // The reply itself is already stored; if even the marker cannot be
        // written there is nothing further to do, and the turn must not fail.
        let _ = log::append(
            store,
            env,
            &req.conversation_id,
            NewEvent::of(EventType::DeliveryFailed),
        );
    }
    json!({"ok": true, "cursor": event.cursor, "delivery_failed": delivery_failed})
}

fn failure(err: &LogError) -> Value {
    let mut value = log::log_error_value(err);
    if let Some(map) = value.as_object_mut() {
        map.insert("ok".into(), Value::Bool(false));
    }
    value
}
