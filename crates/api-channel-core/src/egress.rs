//! Mapping between the host's encoded egress payload and the `send` op.
//!
//! `encode` turns a flow reply into the payload below; `send_payload` turns
//! it back into the `send` input that appends to the conversation log.

use serde_json::{Value, json};

/// The generic code an engine failure is reported as. Provider or engine
/// text is never forwarded to API clients.
pub const FLOW_FAILED: &str = "flow_failed";

/// Build the encoded payload from the pieces of a reply.
pub fn encode_payload(
    session_id: &str,
    text: Option<&str>,
    card: Option<Value>,
    failed: bool,
    awaiting_input: bool,
) -> Value {
    json!({
        "session_id": session_id,
        "text": text,
        "card": card,
        "failed": failed,
        "awaiting_input": awaiting_input,
    })
}

/// Turn an encoded payload into the `send` op input.
///
/// The host folds the deployment's provider config into the payload body; it
/// is carried over so `send` can see the webhook URL.
pub fn send_input(payload: &Value) -> Result<Value, &'static str> {
    let mut input = build_send_input(payload)?;
    if let (Some(config), Some(map)) = (
        payload.get("config").filter(|c| !c.is_null()),
        input.as_object_mut(),
    ) {
        map.insert("config".into(), config.clone());
    }
    Ok(input)
}

fn build_send_input(payload: &Value) -> Result<Value, &'static str> {
    let conversation = payload
        .get("session_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or("session_id required")?;
    if payload.get("failed").and_then(Value::as_bool) == Some(true) {
        return Ok(json!({
            "conversation_id": conversation,
            "type": "error",
            "code": FLOW_FAILED,
        }));
    }
    let text = payload
        .get("text")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty());
    let card = payload.get("card").filter(|c| !c.is_null());
    if text.is_none() && card.is_none() {
        return Err("nothing to send");
    }
    let mut items = Vec::new();
    if let Some(card) = card {
        items.push(json!({
            "kind": "card",
            "card": card,
            "fallback": card_fallback(card, text),
        }));
    } else if let Some(text) = text {
        items.push(json!({"kind": "text", "text": text}));
    }
    let awaiting = payload.get("awaiting_input").and_then(Value::as_bool) == Some(true);
    Ok(json!({
        "conversation_id": conversation,
        "type": if awaiting { "input_required" } else { "message" },
        "items": items,
    }))
}

/// Constant used when neither the card nor the envelope has any text.
pub const CARD_FALLBACK: &str = "Interactive content";

/// Plain-text stand-in for a card: its first non-empty `TextBlock` text, else
/// the envelope text, else [`CARD_FALLBACK`].
fn card_fallback(card: &Value, envelope_text: Option<&str>) -> String {
    first_text_block(card)
        .or_else(|| envelope_text.map(str::to_owned))
        .unwrap_or_else(|| CARD_FALLBACK.to_owned())
}

fn first_text_block(node: &Value) -> Option<String> {
    match node {
        Value::Object(map) => {
            let own = (map.get("type").and_then(Value::as_str) == Some("TextBlock"))
                .then(|| map.get("text").and_then(Value::as_str))
                .flatten()
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(str::to_owned);
            // Content containers first: map keys are sorted, and `actions`
            // sorts ahead of `body`.
            own.or_else(|| {
                ["body", "items", "columns", "facts"]
                    .iter()
                    .filter_map(|key| map.get(*key))
                    .find_map(first_text_block)
            })
            .or_else(|| map.values().find_map(first_text_block))
        }
        Value::Array(items) => items.iter().find_map(first_text_block),
        _ => None,
    }
}

/// The `_greentic` block greentic-start reads from a `send_payload` result to
/// wake the streams of that conversation at once instead of on their next
/// poll. `None` when the send failed or the payload names no tenant.
pub fn notify_metadata(payload: &Value, sent: &Value) -> Option<Value> {
    if sent.get("ok").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let tenant = payload.get("tenant").and_then(Value::as_str)?;
    let conversation = payload.get("session_id").and_then(Value::as_str)?;
    let cursor = sent.get("cursor").and_then(Value::as_u64)?;
    // A `delivery_failed` marker was appended right after the event.
    let bump = u64::from(sent.get("delivery_failed").and_then(Value::as_bool) == Some(true));
    Some(json!({
        "tenant": tenant,
        "conversation_id": conversation,
        "watermark_bumped": cursor + bump,
    }))
}
