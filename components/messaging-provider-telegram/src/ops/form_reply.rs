//! Turning a typed Telegram reply into an Adaptive Card submit.
//!
//! Telegram has no input fields inside a message, so a card with `Input.Text`
//! is sent as text plus a `ForceReply`, and the person answers by replying.
//! The provider is stateless, so what the reply answers has to travel IN the
//! message it replies to: `encode` wraps the first pencil emoji in a
//! `text_link` whose URL carries a [`FormMarker`] (the ordered text input ids
//! and the card's submit routing), and Telegram echoes that entity back in
//! `reply_to_message.entities`. `ingest` reads it and answers as if the card's
//! submit button had been pressed.
//!
//! The marker is read only from a message Telegram says was sent by a bot, so a
//! person cannot forge one: entities of the replied-to message are Telegram's,
//! not the sender's.

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ac_to_html::{AcInput, AcInputKind};

const MARKER_PREFIX: &str = "https://greentic.cloud/tg-form#";
const PENCIL: &str = "\u{270f}\u{fe0f}";

/// What a reply to a form message answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct FormMarker {
    /// Text input ids, in the order the card declared them.
    pub(crate) ids: Vec<String>,
    /// The card's submit data (`routeToCardId` / `cardId`, compacted).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) submit: Option<Value>,
}

impl FormMarker {
    /// `None` when the card has no text input, because then nothing is replied to.
    pub(crate) fn for_inputs(inputs: &[AcInput], submit: Option<Value>) -> Option<Self> {
        let ids: Vec<String> = inputs
            .iter()
            .filter(|i| matches!(i.kind, AcInputKind::Text))
            .map(|i| i.id.clone())
            .collect();
        (!ids.is_empty()).then_some(Self { ids, submit })
    }

    fn url(&self) -> String {
        let json = serde_json::to_vec(self).unwrap_or_default();
        format!("{MARKER_PREFIX}{}", URL_SAFE_NO_PAD.encode(json))
    }

    fn from_url(url: &str) -> Option<Self> {
        let payload = url.strip_prefix(MARKER_PREFIX)?;
        let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
        serde_json::from_slice(&bytes).ok()
    }
}

/// Wrap the first pencil in `html` with the marker link. Unchanged when the
/// html carries no pencil (nothing to anchor the marker on).
pub(crate) fn embed_marker(html: &str, marker: &FormMarker) -> String {
    html.replacen(
        PENCIL,
        &format!("<a href=\"{}\">{PENCIL}</a>", marker.url()),
        1,
    )
}

/// The marker carried by a replied-to message, if any.
pub(crate) fn marker_from_reply(reply_msg: &Value) -> Option<FormMarker> {
    ["entities", "caption_entities"]
        .iter()
        .filter_map(|key| reply_msg.get(*key).and_then(Value::as_array))
        .flatten()
        .filter(|e| e.get("type").and_then(Value::as_str) == Some("text_link"))
        .filter_map(|e| e.get("url").and_then(Value::as_str))
        .find_map(FormMarker::from_url)
}

/// Map a typed reply onto the card's text inputs.
///
/// One input takes the whole text. Several inputs take one non-empty line each,
/// and only when the line count matches: a guess would put an email in the
/// name field, so a mismatch answers `None` and the reply stays plain text.
pub(crate) fn answers(ids: &[String], text: &str) -> Option<Vec<(String, String)>> {
    match ids {
        [] => None,
        [only] => {
            let value = text.trim();
            (!value.is_empty()).then(|| vec![(only.clone(), value.to_string())])
        }
        many => {
            let lines: Vec<&str> = text
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .collect();
            (lines.len() == many.len()).then(|| {
                many.iter()
                    .cloned()
                    .zip(lines.into_iter().map(str::to_string))
                    .collect()
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn marker() -> FormMarker {
        FormMarker {
            ids: vec!["name".into(), "email".into()],
            submit: Some(json!({"r": "thanks"})),
        }
    }

    #[test]
    fn marker_survives_the_trip_through_a_telegram_entity() {
        let html = embed_marker(
            "\u{270f}\u{fe0f} <b>Name</b>\n\u{270f}\u{fe0f} <b>Email</b>",
            &marker(),
        );
        assert_eq!(
            html.matches("<a href=").count(),
            1,
            "only the first pencil is linked"
        );
        let url = html.split('"').nth(1).expect("href");
        let reply =
            json!({"entities": [{"type": "text_link", "offset": 0, "length": 2, "url": url}]});
        assert_eq!(marker_from_reply(&reply), Some(marker()));
    }

    #[test]
    fn a_foreign_link_is_not_a_marker() {
        let reply = json!({"entities": [{"type": "text_link", "url": "https://example.com/x"}]});
        assert_eq!(marker_from_reply(&reply), None);
        assert_eq!(marker_from_reply(&json!({})), None);
    }

    #[test]
    fn a_card_without_text_inputs_has_no_marker() {
        assert_eq!(FormMarker::for_inputs(&[], None), None);
    }

    #[test]
    fn one_input_takes_the_whole_reply() {
        let got = answers(&["name".into()], "  Bima Pangestu \n").expect("answers");
        assert_eq!(got, vec![("name".to_string(), "Bima Pangestu".to_string())]);
    }

    #[test]
    fn several_inputs_take_one_line_each() {
        let ids = vec!["name".to_string(), "email".to_string()];
        let got = answers(&ids, "Bima Pangestu\nbima@x.id").expect("answers");
        assert_eq!(got[0], ("name".into(), "Bima Pangestu".into()));
        assert_eq!(got[1], ("email".into(), "bima@x.id".into()));
    }

    #[test]
    fn a_line_count_mismatch_is_not_guessed() {
        let ids = vec!["name".to_string(), "email".to_string()];
        assert_eq!(answers(&ids, "only one line"), None);
        assert_eq!(answers(&ids, "a\nb\nc"), None);
    }
}
