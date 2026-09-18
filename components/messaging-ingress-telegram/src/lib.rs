mod bindings {
    wit_bindgen::generate!({
        path: "wit/messaging-ingress-telegram",
        world: "messaging-ingress-telegram",
        generate_all
    });
}

use bindings::exports::provider::common0_0_2::ingress::Guest;
use bindings::exports::provider::common0_0_3::ingress::Guest as ConfiguredIngressGuest;
use serde_json::{Map, Value, json};

struct Component;

impl Guest for Component {
    fn handle_webhook(_headers_json: String, body_json: String) -> Result<String, String> {
        let parsed: Value = serde_json::from_str(&body_json)
            .map_err(|_| "validation error: invalid body".to_string())?;

        let mut events = Vec::new();
        if let Some(envelope) = envelope_from_update(&parsed) {
            events.push(envelope);
        }

        // `events` must be present and plural: the host reads only "events"/"emitted_events",
        // and its WIT-envelope unwrap discards the payload when "events" is absent.
        let normalized = json!({
            "ok": true,
            "event": parsed,
            "events": events,
        });
        serde_json::to_string(&normalized)
            .map_err(|_| "other error: serialization failed".to_string())
    }
}

/// Build a `ChannelMessageEnvelope` from a Telegram update.
///
/// Returns `None` for updates that carry no actionable message — bot-authored messages, and
/// update kinds we do not route (edits, channel posts, polls).
fn envelope_from_update(update: &Value) -> Option<Value> {
    let update_id = update.get("update_id").and_then(Value::as_i64);

    // A button press on an Adaptive Card arrives as `callback_query`, with the originating
    // chat nested under `message`.
    if let Some(callback) = update.get("callback_query") {
        let text = callback
            .get("data")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let chat = callback.get("message").and_then(|m| m.get("chat"));
        return Some(envelope_from_parts(
            text,
            chat_id(chat).as_deref(),
            sender_id(callback.get("from")).as_deref(),
            update_id,
        ));
    }

    let message = update.get("message")?;
    if is_bot_message(message) {
        return None;
    }

    let text = message
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default();
    Some(envelope_from_parts(
        text,
        chat_id(message.get("chat")).as_deref(),
        sender_id(message.get("from")).as_deref(),
        update_id,
    ))
}

fn is_bot_message(message: &Value) -> bool {
    message
        .get("from")
        .and_then(|from| from.get("is_bot"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Telegram ids are JSON numbers; normalize to string so they survive as channel/session keys.
fn chat_id(chat: Option<&Value>) -> Option<String> {
    id_as_string(chat?.get("id")?)
}

fn sender_id(from: Option<&Value>) -> Option<String> {
    id_as_string(from?.get("id")?)
}

fn id_as_string(value: &Value) -> Option<String> {
    match value {
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
        _ => None,
    }
}

fn envelope_from_parts(
    text: &str,
    chat: Option<&str>,
    sender: Option<&str>,
    update_id: Option<i64>,
) -> Value {
    let chat = chat.map(ToOwned::to_owned);
    let sender = sender.map(ToOwned::to_owned);
    let channel_name = chat.clone().unwrap_or_else(|| "telegram".to_string());
    let envelope_id = update_id
        .map(|id| format!("telegram:{id}"))
        .unwrap_or_else(|| format!("telegram-{channel_name}"));

    let mut metadata = Map::new();
    metadata.insert("universal".to_string(), Value::String("true".to_string()));
    if let Some(chat_id) = chat.as_ref() {
        metadata.insert("channel".to_string(), Value::String(chat_id.clone()));
    }
    if let Some(sender_id) = sender.as_ref() {
        metadata.insert("from".to_string(), Value::String(sender_id.clone()));
    }
    if let Some(id) = update_id {
        metadata.insert("update_id".to_string(), Value::String(id.to_string()));
    }

    json!({
        "id": envelope_id,
        "tenant": {
            "env": "default",
            "tenant": "default",
            "tenant_id": "default",
            "attempt": 0
        },
        "channel": channel_name,
        "session_id": chat.clone().unwrap_or_else(|| "telegram".to_string()),
        "from": sender.map(|id| json!({"id": id, "kind": "user"})),
        "to": chat.map(|id| vec![json!({"id": id})]).unwrap_or_default(),
        "text": text,
        "attachments": [],
        "metadata": metadata,
    })
}

impl ConfiguredIngressGuest for Component {
    fn handle_webhook(
        headers_json: String,
        body_json: String,
        _config_json: String,
    ) -> Result<String, String> {
        <Component as Guest>::handle_webhook(headers_json, body_json)
    }
}

bindings::exports::provider::common0_0_2::ingress::__export_provider_common_ingress_0_0_2_cabi!(
    Component with_types_in bindings::exports::provider::common0_0_2::ingress
);
bindings::exports::provider::common0_0_3::ingress::__export_provider_common_ingress_0_0_3_cabi!(
    Component with_types_in bindings::exports::provider::common0_0_3::ingress
);

#[cfg(test)]
mod tests {
    use super::*;

    fn handle(body: &str) -> Value {
        let out = <Component as Guest>::handle_webhook("{}".to_string(), body.to_string())
            .expect("normalized");
        serde_json::from_str(&out).expect("json")
    }

    #[test]
    fn configured_export_matches_the_unconfigured_one() {
        let body = r#"{"update_id":1,"message":{"text":"hi","chat":{"id":7}}}"#;
        let plain = <Component as Guest>::handle_webhook("{}".into(), body.into());
        let configured = <Component as ConfiguredIngressGuest>::handle_webhook(
            "{}".into(),
            body.into(),
            r#"{"public_base_url":"https://example.com"}"#.into(),
        );
        assert_eq!(plain, configured);
    }

    #[test]
    fn webhook_wraps_update_json_in_normalized_event() {
        let parsed = handle(r#"{"update_id":1,"message":{"text":"hello"}}"#);

        assert_eq!(parsed["ok"], true);
        assert_eq!(parsed["event"]["update_id"], 1);
        assert_eq!(parsed["event"]["message"]["text"], "hello");
    }

    #[test]
    fn webhook_emits_plural_events_array() {
        // Regression: the host reads only "events"/"emitted_events". Emitting just "event"
        // caused every inbound update to be silently discarded with a 200 response.
        let parsed = handle(
            r#"{"update_id":42,"message":{"message_id":7,"chat":{"id":777,"type":"private"},
                "from":{"id":555,"is_bot":false},"text":"weather in paris"}}"#,
        );

        let events = parsed["events"].as_array().expect("events array");
        assert_eq!(events.len(), 1);
        let envelope = &events[0];
        assert_eq!(envelope["text"], "weather in paris");
        assert_eq!(envelope["id"], "telegram:42");
        assert_eq!(envelope["channel"], "777");
        assert_eq!(envelope["session_id"], "777");
        assert_eq!(envelope["from"]["id"], "555");
        assert_eq!(envelope["from"]["kind"], "user");
        assert_eq!(envelope["to"][0]["id"], "777");
        assert_eq!(envelope["metadata"]["universal"], "true");
        assert_eq!(envelope["metadata"]["update_id"], "42");
    }

    #[test]
    fn webhook_routes_callback_query_button_press() {
        let parsed = handle(
            r#"{"update_id":9,"callback_query":{"id":"cb1","data":"action:refresh",
                "from":{"id":555,"is_bot":false},
                "message":{"chat":{"id":777,"type":"private"}}}}"#,
        );

        let events = parsed["events"].as_array().expect("events array");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["text"], "action:refresh");
        assert_eq!(events[0]["channel"], "777");
        assert_eq!(events[0]["from"]["id"], "555");
    }

    #[test]
    fn webhook_suppresses_bot_authored_messages() {
        let parsed = handle(
            r#"{"update_id":3,"message":{"chat":{"id":777,"type":"private"},
                "from":{"id":999,"is_bot":true},"text":"echo"}}"#,
        );

        assert_eq!(parsed["ok"], true);
        assert!(
            parsed["events"]
                .as_array()
                .expect("events array")
                .is_empty(),
            "bot-authored messages must not produce envelopes"
        );
    }

    #[test]
    fn webhook_ignores_unroutable_update_kinds() {
        // No `message` and no `callback_query` — e.g. a poll or edited channel post.
        let parsed = handle(r#"{"update_id":5,"poll":{"id":"p1"}}"#);

        assert_eq!(parsed["ok"], true);
        assert!(
            parsed["events"]
                .as_array()
                .expect("events array")
                .is_empty()
        );
    }

    #[test]
    fn webhook_rejects_invalid_json() {
        let err = <Component as Guest>::handle_webhook("{}".to_string(), "{".to_string())
            .expect_err("invalid body should fail");

        assert!(err.contains("invalid body"), "{err}");
    }
}
