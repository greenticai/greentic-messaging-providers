//! `ingest_http` op for the Telegram provider.
//!
//! Normalises a raw Telegram webhook body into a
//! [`ChannelMessageEnvelope`] + HTTP response envelope. Supports both regular
//! `message` events (including reply-to-bot detection that flips the envelope
//! into form-input mode) and `callback_query` events from inline keyboard
//! button clicks. Shared envelope + extractor helpers live in this file so
//! that `send.rs` and `send_payload.rs` can reuse them.

use base64::{Engine, engine::general_purpose::STANDARD};
use greentic_types::messaging::universal_dto::HttpOutV1;
use greentic_types::{
    Actor, ChannelMessageEnvelope, Destination, EnvId, MessageMetadata, TenantCtx, TenantId,
};
use provider_common::http_compat::{http_out_error, http_out_v1_bytes, parse_operator_http_in};
use provider_common::lifecycle_events::{mark_user_entered, user_entered_idempotency_key};
use serde_json::{Value, json};

use super::form_reply;

fn debug_enabled() -> bool {
    matches!(
        std::env::var("TELEGRAM_DEBUG")
            .or_else(|_| std::env::var("GREENTIC_DEBUG"))
            .as_deref(),
        Ok("1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON")
    )
}

macro_rules! telegram_debug {
    ($($arg:tt)*) => {
        if debug_enabled() {
            provider_common::telemetry::log(
                provider_common::telemetry::Level::Debug,
                &format!($($arg)*),
                &[provider_common::telemetry::Field {
                    key: provider_common::telemetry::field::PROVIDER,
                    value: "telegram",
                }],
            );
        }
    };
}

pub(crate) fn ingest_http(input_json: &[u8]) -> Vec<u8> {
    // Use operator-compat parser which handles both `body_b64` (string)
    // and `body` (raw byte array from operator's IngressRequestV1).
    let request = match parse_operator_http_in(input_json) {
        Ok(req) => req,
        Err(err) => return http_out_error(400, &format!("invalid http input: {err}")),
    };
    let body_bytes = match STANDARD.decode(&request.body_b64) {
        Ok(bytes) => bytes,
        Err(err) => return http_out_error(400, &format!("invalid body encoding: {err}")),
    };
    let body_val: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);

    // Handle callback_query (inline keyboard button clicks — e.g. AC Action.Submit).
    let has_callback = body_val.get("callback_query").is_some();
    let has_message = body_val.get("message").is_some();
    telegram_debug!(
        "telegram ingest_http: has_callback={} has_message={} keys={:?}",
        has_callback,
        has_message,
        body_val
            .as_object()
            .map(|o| o.keys().collect::<Vec<_>>())
            .unwrap_or_default()
    );
    if let Some(callback) = body_val.get("callback_query") {
        return ingest_callback_query(&body_val, callback);
    }

    let message = body_val.get("message").cloned().unwrap_or(Value::Null);
    let text = extract_message_text(&message);
    let chat_id = extract_chat_id(&message);
    let from = extract_from_user(&message);
    let msg_locale = extract_language_code(&message);
    let mut envelope = build_telegram_envelope_with_locale(
        text.clone(),
        chat_id.clone(),
        from.clone(),
        msg_locale,
    );
    if is_start_command(&text) {
        let idempotency_key = user_entered_idempotency_key(
            "telegram",
            None,
            chat_id.as_deref(),
            from.as_deref(),
            "start_command",
        );
        mark_user_entered(
            &mut envelope.metadata,
            "telegram",
            "start_command",
            idempotency_key,
        );
        if let Some(chat) = &chat_id {
            envelope
                .metadata
                .insert("chat_id".to_string(), chat.clone());
        }
        if let Some(sender) = &from {
            envelope
                .metadata
                .insert("user_id".to_string(), sender.clone());
        }
    }

    // Detect reply-to-bot messages (form input responses from ForceReply).
    // When user replies to a bot message that had a form prompt, mark the
    // envelope so the flow engine can treat it as form input.
    if let Some(reply_msg) = message.get("reply_to_message") {
        let is_from_bot = reply_msg
            .get("from")
            .and_then(|f| f.get("is_bot"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if is_from_bot {
            envelope
                .metadata
                .insert("is_form_reply".to_string(), "true".to_string());
            // Pass the original bot message_id for context.
            if let Some(mid) = reply_msg.get("message_id").and_then(Value::as_i64) {
                envelope
                    .metadata
                    .insert("reply_to_bot_message_id".to_string(), mid.to_string());
            }
            submit_form_reply(&mut envelope, &text, reply_msg);
        }
    }
    let normalized = json!({
        "ok": true,
        "event": body_val,
        "message": message,
        "chat_id": chat_id,
        "from": from
    });
    let normalized_bytes = serde_json::to_vec(&normalized).unwrap_or_else(|_| b"{}".to_vec());
    let out = HttpOutV1 {
        status: 200,
        headers: Vec::new(),
        body_b64: STANDARD.encode(&normalized_bytes),
        events: vec![envelope],
    };
    http_out_v1_bytes(&out)
}

fn ingest_callback_query(body_val: &Value, callback: &Value) -> Vec<u8> {
    let callback_id = callback
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let cb_message = callback.get("message").unwrap_or(&Value::Null);
    let chat_id = extract_chat_id(cb_message);
    let from = extract_from_user(callback);
    let data_str = callback
        .get("data")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    telegram_debug!(
        "telegram callback_query: id={} data_str={}",
        callback_id,
        data_str
    );

    // Try to parse callback data as JSON (AC Action.Submit serializes data as JSON).
    let (route_to_card, card_id, action_text) = parse_callback_data(data_str);

    let cb_locale = extract_language_code(callback);
    let mut envelope =
        build_telegram_envelope_with_locale(action_text, chat_id.clone(), from.clone(), cb_locale);
    if !route_to_card.is_empty() {
        envelope
            .metadata
            .insert("routeToCardId".into(), route_to_card);
    }
    if !card_id.is_empty() {
        envelope.metadata.insert("cardId".into(), card_id);
    }
    envelope
        .metadata
        .insert("callback_query_id".into(), callback_id.clone());
    envelope
        .metadata
        .insert("callback_data".into(), data_str.to_string());
    // Flatten the AC Action.Submit data fields into metadata (mirrors
    // slack/teams/webex ingest) so the flow can route on e.g.
    // `response.action`. Telegram's 64-byte callback_data round-trips the
    // submit data as a JSON string, so the action would otherwise stay buried
    // inside `callback_data` and never reach the flow's routing context.
    //
    // `or_insert`, not `insert`: callback_data is client-supplied, so a crafted
    // value must never overwrite the trusted metadata already derived from the
    // authenticated Telegram callback fields (chat_id, from, routeToCardId,
    // ...). New keys like `action` still flow; existing trusted keys win.
    if let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(data_str) {
        for (k, v) in obj {
            let s = match v {
                Value::String(s) => s,
                other => other.to_string(),
            };
            envelope.metadata.entry(k).or_insert(s);
        }
    }

    let normalized = json!({
        "ok": true,
        "event": body_val,
        "callback_query": callback,
        "chat_id": chat_id,
        "from": from,
    });
    let normalized_bytes = serde_json::to_vec(&normalized).unwrap_or_else(|_| b"{}".to_vec());
    let out = HttpOutV1 {
        status: 200,
        headers: Vec::new(),
        body_b64: STANDARD.encode(&normalized_bytes),
        events: vec![envelope],
    };
    http_out_v1_bytes(&out)
}

/// Answer the card a typed reply responds to, as if its submit button had been
/// pressed: the typed values become metadata (so they reach the card's
/// `answers`) and the card's routing is restored. A reply that cannot be mapped
/// onto the card's inputs is left as the plain form reply it was.
///
/// `or_insert`, not `insert`: input ids come from the card, and the trusted
/// metadata derived from the authenticated Telegram update must win.
fn submit_form_reply(envelope: &mut ChannelMessageEnvelope, text: &str, reply_msg: &Value) {
    let Some(marker) = form_reply::marker_from_reply(reply_msg) else {
        return;
    };
    let Some(answers) = form_reply::answers(&marker.ids, text) else {
        return;
    };
    let submit_str = marker
        .submit
        .as_ref()
        .map(Value::to_string)
        .unwrap_or_default();
    let (route_to_card, card_id, action_text) = parse_callback_data(&submit_str);
    if !route_to_card.is_empty() {
        envelope
            .metadata
            .insert("routeToCardId".into(), route_to_card);
    }
    if !card_id.is_empty() {
        envelope.metadata.insert("cardId".into(), card_id);
    }
    if !submit_str.is_empty() {
        envelope.text = Some(action_text);
    }
    for (id, value) in answers {
        envelope.metadata.entry(id).or_insert(value);
    }
}

/// Parse callback data which AC Action.Submit serialises as JSON.
/// Supports both full keys (routeToCardId/cardId) and abbreviated keys (r/c)
/// used by `compact_callback_data` to fit Telegram's 64-byte limit.
fn parse_callback_data(data_str: &str) -> (String, String, String) {
    if let Ok(data_val) = serde_json::from_str::<Value>(data_str) {
        let rtc = data_val
            .get("routeToCardId")
            .or_else(|| data_val.get("r"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let cid = data_val
            .get("cardId")
            .or_else(|| data_val.get("c"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let text = if !rtc.is_empty() {
            format!("[card:{rtc}]")
        } else if !cid.is_empty() {
            format!("[action:{cid}]")
        } else {
            format!("[callback:{data_str}]")
        };
        (rtc, cid, text)
    } else {
        (
            String::new(),
            String::new(),
            format!("[callback:{data_str}]"),
        )
    }
}

fn build_telegram_envelope_with_locale(
    text: String,
    chat_id: Option<String>,
    from: Option<String>,
    locale: Option<String>,
) -> ChannelMessageEnvelope {
    let env = EnvId::try_from("default").expect("env id");
    let tenant = TenantId::try_from("default").expect("tenant id");
    let mut metadata = MessageMetadata::new();
    metadata.insert("universal".to_string(), "true".to_string());
    if let Some(chat) = &chat_id {
        metadata.insert("chat_id".to_string(), chat.clone());
    }
    if let Some(sender) = &from {
        metadata.insert("from".to_string(), sender.clone());
    }
    if let Some(lang) = &locale
        && !lang.is_empty()
    {
        metadata.insert("locale".to_string(), lang.clone());
    }
    let channel = "telegram".to_string();
    let sender = from.map(|id| Actor {
        id,
        kind: Some("user".into()),
    });
    let destinations = if let Some(chat) = &chat_id {
        vec![Destination {
            id: chat.clone(),
            kind: Some("chat".into()),
        }]
    } else {
        Vec::new()
    };
    ChannelMessageEnvelope {
        id: format!("telegram-{channel}"),
        tenant: TenantCtx::new(env.clone(), tenant.clone()),
        channel: channel.clone(),
        session_id: chat_id.clone().unwrap_or_else(|| "telegram".to_string()),
        reply_scope: None,
        from: sender,
        to: destinations,
        correlation_id: None,
        text: Some(text),
        attachments: Vec::new(),
        metadata,
        extensions: Default::default(),
    }
}

fn is_start_command(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed == "/start" || trimmed.starts_with("/start ")
}

pub(crate) fn extract_message_text(value: &Value) -> String {
    value
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

pub(crate) fn extract_chat_id(value: &Value) -> Option<String> {
    value
        .get("chat")
        .and_then(|chat| chat.get("id"))
        .and_then(Value::as_i64)
        .map(|id| id.to_string())
}

pub(crate) fn extract_from_user(value: &Value) -> Option<String> {
    value
        .get("from")
        .and_then(|from| from.get("id"))
        .and_then(Value::as_i64)
        .map(|id| id.to_string())
}

/// Extract the user's preferred language from the Telegram `from.language_code` field.
fn extract_language_code(value: &Value) -> Option<String> {
    value
        .get("from")
        .and_then(|from| from.get("language_code"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

pub(crate) fn extract_ids(body: &Value) -> (String, String) {
    let message_id = body
        .get("result")
        .and_then(|v| v.get("message_id"))
        .map(|val| match val {
            Value::Number(num) => num.to_string(),
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .unwrap_or_else(|| "dummy-message-id".into());
    let provider_message_id = format!("tg:{message_id}");
    (message_id, provider_message_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use greentic_types::messaging::universal_dto::{HttpInV1, HttpOutV1};
    use serde_json::json;

    fn ingest_body(body: Value) -> HttpOutV1 {
        let request = HttpInV1 {
            method: "POST".to_string(),
            path: "/telegram".to_string(),
            query: None,
            headers: Vec::new(),
            body_b64: STANDARD.encode(serde_json::to_vec(&body).expect("body json")),
            binding_id: None,
            route_hint: None,
            config: None,
        };
        let bytes = ingest_http(&serde_json::to_vec(&request).expect("request json"));
        serde_json::from_slice(&bytes).expect("http out")
    }

    #[test]
    fn callback_query_accepts_compact_card_routing_fields() {
        let out = ingest_body(json!({
            "callback_query": {
                "id": "cb-1",
                "from": {"id": 42, "language_code": "nl"},
                "message": {"chat": {"id": 99}},
                "data": "{\"r\":\"card-route\",\"c\":\"button-1\"}"
            }
        }));

        assert_eq!(out.status, 200);
        assert_eq!(out.events.len(), 1);
        let event = &out.events[0];
        assert_eq!(event.session_id, "99");
        assert_eq!(event.text.as_deref(), Some("[card:card-route]"));
        assert_eq!(
            event.metadata.get("routeToCardId").map(String::as_str),
            Some("card-route")
        );
        assert_eq!(
            event.metadata.get("cardId").map(String::as_str),
            Some("button-1")
        );
        assert_eq!(event.metadata.get("locale").map(String::as_str), Some("nl"));
    }

    #[test]
    fn callback_query_flattens_submit_action_without_overwriting_trusted_metadata() {
        // AC Action.Submit data under 64 bytes round-trips as full JSON (not
        // compacted to r/c). Flatten its fields into metadata so the flow can
        // route on `response.action`. Client-supplied keys that collide with
        // trusted metadata (chat_id, from) derived from the authenticated
        // Telegram fields must NOT be overwritten.
        let out = ingest_body(json!({
            "callback_query": {
                "id": "cb-1",
                "from": {"id": 42},
                "message": {"chat": {"id": 99}},
                "data": "{\"action\":\"about_card\",\"chat_id\":\"99999\",\"from\":\"attacker\"}"
            }
        }));

        assert_eq!(out.status, 200);
        let event = &out.events[0];
        // New (non-colliding) submit field flows through for routing.
        assert_eq!(
            event.metadata.get("action").map(String::as_str),
            Some("about_card")
        );
        // Trusted identity/session keys keep their authenticated values.
        assert_eq!(
            event.metadata.get("chat_id").map(String::as_str),
            Some("99")
        );
        assert_eq!(event.metadata.get("from").map(String::as_str), Some("42"));
        // Raw callback_data is still preserved alongside the flattened fields.
        assert_eq!(
            event.metadata.get("callback_data").map(String::as_str),
            Some("{\"action\":\"about_card\",\"chat_id\":\"99999\",\"from\":\"attacker\"}")
        );
    }

    #[test]
    fn reply_to_bot_marks_form_reply_context() {
        let out = ingest_body(json!({
            "message": {
                "message_id": 12,
                "text": "Blue",
                "chat": {"id": 99},
                "from": {"id": 42, "language_code": "en"},
                "reply_to_message": {
                    "message_id": 11,
                    "from": {"is_bot": true}
                }
            }
        }));

        assert_eq!(out.status, 200);
        let event = &out.events[0];
        assert_eq!(event.text.as_deref(), Some("Blue"));
        assert_eq!(
            event.metadata.get("is_form_reply").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            event
                .metadata
                .get("reply_to_bot_message_id")
                .map(String::as_str),
            Some("11")
        );
    }

    fn form_marker_url(ids: &[&str], submit: Value) -> String {
        let html = form_reply::embed_marker(
            "\u{270f}\u{fe0f} <b>Full name</b>",
            &form_reply::FormMarker {
                ids: ids.iter().map(|s| s.to_string()).collect(),
                submit: Some(submit),
            },
        );
        html.split('"').nth(1).expect("href").to_string()
    }

    #[test]
    fn a_typed_reply_submits_the_card_it_answers() {
        let url = form_marker_url(&["name", "email"], json!({"r": "thanks"}));
        let out = ingest_body(json!({
            "message": {
                "message_id": 12,
                "text": "Bima Pangestu\nbima@x.id",
                "chat": {"id": 99},
                "from": {"id": 42},
                "reply_to_message": {
                    "message_id": 11,
                    "from": {"is_bot": true},
                    "entities": [{"type": "text_link", "offset": 0, "length": 2, "url": url}]
                }
            }
        }));

        let event = &out.events[0];
        let meta = |k: &str| event.metadata.get(k).map(String::as_str);
        assert_eq!(meta("name"), Some("Bima Pangestu"));
        assert_eq!(meta("email"), Some("bima@x.id"));
        assert_eq!(meta("routeToCardId"), Some("thanks"));
        assert_eq!(event.text.as_deref(), Some("[card:thanks]"));
    }

    #[test]
    fn a_reply_that_does_not_fit_the_inputs_stays_a_plain_form_reply() {
        let url = form_marker_url(&["name", "email"], json!({"r": "thanks"}));
        let out = ingest_body(json!({
            "message": {
                "message_id": 12,
                "text": "just one line",
                "chat": {"id": 99},
                "from": {"id": 42},
                "reply_to_message": {
                    "message_id": 11,
                    "from": {"is_bot": true},
                    "entities": [{"type": "text_link", "url": url}]
                }
            }
        }));

        let event = &out.events[0];
        assert_eq!(event.text.as_deref(), Some("just one line"));
        assert!(!event.metadata.contains_key("name"));
        assert!(!event.metadata.contains_key("routeToCardId"));
    }

    #[test]
    fn an_answer_never_overwrites_trusted_metadata() {
        let url = form_marker_url(&["chat_id"], json!({"r": "thanks"}));
        let out = ingest_body(json!({
            "message": {
                "message_id": 12,
                "text": "attacker",
                "chat": {"id": 99},
                "from": {"id": 42},
                "reply_to_message": {
                    "message_id": 11,
                    "from": {"is_bot": true},
                    "entities": [{"type": "text_link", "url": url}]
                }
            }
        }));

        assert_eq!(
            out.events[0].metadata.get("chat_id").map(String::as_str),
            Some("99")
        );
    }

    #[test]
    fn start_command_marks_user_entered_lifecycle_event() {
        let out = ingest_body(json!({
            "message": {
                "message_id": 12,
                "text": "/start demo",
                "chat": {"id": 99},
                "from": {"id": 42, "language_code": "en"}
            }
        }));

        assert_eq!(out.status, 200);
        let event = &out.events[0];
        assert_eq!(
            event.metadata.get("event_type").map(String::as_str),
            Some("channel.user.entered")
        );
        assert_eq!(
            event.metadata.get("autoStart").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            event.metadata.get("provider").map(String::as_str),
            Some("telegram")
        );
        assert_eq!(
            event.metadata.get("reason").map(String::as_str),
            Some("start_command")
        );
        assert_eq!(
            event.metadata.get("idempotency_key").map(String::as_str),
            Some("lifecycle.user_entered:telegram:_:99:42:start_command")
        );
    }

    #[test]
    fn extract_ids_preserves_string_message_ids() {
        let (message_id, provider_message_id) =
            extract_ids(&json!({"result": {"message_id": "abc-123"}}));

        assert_eq!(message_id, "abc-123");
        assert_eq!(provider_message_id, "tg:abc-123");
    }
}
