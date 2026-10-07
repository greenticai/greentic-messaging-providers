//! WhatsApp Cloud API webhook -> messages + fetch references (master plan C1).
//!
//! Shared by the provider `ingest_http` so the mapping lives in one place.
//!
//! The reference is `{kind: "whatsapp_media", media_id}` and never a URL: the
//! host does the two-step download (media id -> short-lived URL -> bytes, both
//! with the bearer token), so the provider never builds a Graph URL or touches
//! the token.
//!
//! Every message of every entry and change in an update is read, in order.
//! Only `image` and `document` media are mapped; audio, video, sticker and
//! location messages are out of v1 scope and yield a message with no
//! attachment (they do not break the envelope).
//!
//! Assumed payload shapes (API knowledge, not verified against this repo):
//! `entry[].changes[].value.messages[]` with `id` (`wamid.…`), `from`, `type`;
//! `image{id, mime_type, sha256?, caption?}`;
//! `document{id, filename?, mime_type, caption?}`; `text{body}`.

use crate::attachment_fetch::{FetchRef, PendingAttachment};
use serde_json::Value;

/// One inbound WhatsApp message.
#[derive(Debug, Clone)]
pub struct WhatsappMessage {
    /// The WhatsApp message id (`wamid.…`), when present.
    pub id: Option<String>,
    pub from: Option<String>,
    /// `text.body`, else the media caption, else empty.
    pub text: String,
    pub pending: Vec<PendingAttachment>,
}

/// All messages of a webhook body, in order. A body with no Cloud API messages
/// is read as the flat legacy format (the body itself is the one message).
pub fn parse_messages(body: &Value) -> Vec<WhatsappMessage> {
    let cloud: Vec<&Value> = body
        .get("entry")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|e| {
            e.get("changes")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(|c| c.get("value")?.get("messages")?.as_array())
        .flatten()
        .collect();
    if cloud.is_empty() {
        return vec![message_from(body)];
    }
    cloud.into_iter().map(message_from).collect()
}

fn message_from(msg: &Value) -> WhatsappMessage {
    let mut pending = Vec::new();
    let mut caption = None;
    for kind in ["image", "document"] {
        let Some(media) = msg.get(kind) else { continue };
        // The caption is the user's text even when the media itself is unusable.
        caption = caption.or_else(|| media.get("caption").and_then(Value::as_str));
        let Some(media_id) = media
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        pending.push(PendingAttachment {
            mime_type: media
                .get("mime_type")
                .and_then(Value::as_str)
                .unwrap_or("application/octet-stream")
                .to_string(),
            name: media
                .get("filename")
                .and_then(Value::as_str)
                .map(str::to_string),
            size_bytes: None,
            fetch: FetchRef::WhatsappMedia {
                media_id: media_id.to_string(),
            },
            inline_base64: None,
        });
    }
    let text = msg
        .get("text")
        .and_then(|t| t.get("body"))
        .and_then(Value::as_str)
        .or_else(|| msg.get("text").and_then(Value::as_str))
        .or(caption)
        .unwrap_or("")
        .to_string();
    WhatsappMessage {
        id: msg
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        from: msg.get("from").and_then(Value::as_str).map(str::to_string),
        text,
        pending,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn body() -> Value {
        let v: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/whatsapp/inbound/image_message.json"
        ))
        .expect("fixture");
        v["body"].clone()
    }

    #[test]
    fn every_message_in_the_update_is_read() {
        let msgs = parse_messages(&body());
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].text, "ini apa?");
        assert_eq!(msgs[2].text, "halo");
        assert_eq!(msgs[1].id.as_deref(), Some("wamid.B"));
    }

    #[test]
    fn media_messages_become_whatsapp_media_refs() {
        let msgs = parse_messages(&body());
        assert_eq!(
            msgs[0].pending[0].fetch,
            FetchRef::WhatsappMedia {
                media_id: "MEDIA1".into()
            }
        );
        assert_eq!(msgs[0].pending[0].mime_type, "image/jpeg");
        assert_eq!(msgs[1].pending[0].name.as_deref(), Some("x.pdf"));
        assert!(msgs[2].pending.is_empty());
    }

    #[test]
    fn all_entries_and_changes_are_walked() {
        let b = json!({"entry":[
            {"changes":[{"value":{"messages":[{"id":"1","text":{"body":"a"}}]}},
                        {"value":{"messages":[{"id":"2","text":{"body":"b"}}]}}]},
            {"changes":[{"value":{"messages":[{"id":"3","text":{"body":"c"}}]}}]}]});
        let texts: Vec<_> = parse_messages(&b).into_iter().map(|m| m.text).collect();
        assert_eq!(texts, ["a", "b", "c"]);
    }

    #[test]
    fn unsupported_media_keeps_the_message_without_attachment() {
        let b = json!({"entry":[{"changes":[{"value":{"messages":[
            {"id":"w","from":"1","type":"audio","audio":{"id":"A1","mime_type":"audio/ogg"}}]}}]}]});
        let msgs = parse_messages(&b);
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].pending.is_empty());
        assert_eq!(msgs[0].text, "");
    }

    #[test]
    fn media_without_an_id_is_skipped() {
        let b = json!({"entry":[{"changes":[{"value":{"messages":[
            {"id":"w","type":"image","image":{"mime_type":"image/png","caption":"c"}}]}}]}]});
        let msgs = parse_messages(&b);
        assert!(msgs[0].pending.is_empty());
        assert_eq!(msgs[0].text, "c");
    }

    #[test]
    fn flat_legacy_format_still_parses() {
        let msgs = parse_messages(&json!({"from":"1","text":{"body":"hi"}}));
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text, "hi");
        assert_eq!(msgs[0].from.as_deref(), Some("1"));
        assert_eq!(msgs[0].id, None);
    }

    #[test]
    fn a_text_body_wins_over_a_caption() {
        let b = json!({"text":{"body":"t"},"image":{"id":"I","caption":"c"}});
        assert_eq!(parse_messages(&b)[0].text, "t");
    }
}
