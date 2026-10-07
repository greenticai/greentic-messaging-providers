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

/// Most messages processed from one update; the rest are dropped and counted.
pub const MAX_MESSAGES_PER_UPDATE: usize = 100;

/// Longest `wamid` kept readable in an envelope id.
const MAX_READABLE_ID_LEN: usize = 128;

/// The messages of one webhook body plus how many were dropped by the cap.
#[derive(Debug, Clone)]
pub struct ParsedUpdate {
    pub messages: Vec<WhatsappMessage>,
    pub dropped: usize,
}

/// Envelope id for a WhatsApp message id: `whatsapp-<wamid>` when the trimmed
/// wamid is 1..=128 chars of `[A-Za-z0-9._=+-]` (real ids look like
/// `wamid.HBgM…`); otherwise `whatsapp-~<first 32 hex of sha256(trimmed
/// wamid)>`. Deterministic; `~` is outside the readable alphabet, so a hashed
/// id can never equal a readable one and no wamid can forge another's hash id.
pub fn whatsapp_envelope_id(wamid: &str) -> String {
    use sha2::{Digest, Sha256};
    let id = wamid.trim();
    let readable = (1..=MAX_READABLE_ID_LEN).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'=' | b'+' | b'-'));
    if readable {
        return format!("whatsapp-{id}");
    }
    let digest = Sha256::digest(id.as_bytes());
    let hex: String = digest.iter().take(16).map(|b| format!("{b:02x}")).collect();
    format!("whatsapp-~{hex}")
}

/// All messages of a webhook body (at most [`MAX_MESSAGES_PER_UPDATE`]).
pub fn parse_messages(body: &Value) -> Vec<WhatsappMessage> {
    parse_update(body).messages
}

/// Messages of a webhook body, in order, with the dropped count.
///
/// The flat legacy format (the body itself is the message) applies ONLY when
/// the body has no `entry` key. A Cloud API update that carries no messages
/// (delivery receipts, `statuses` only, malformed `messages`) yields nothing:
/// it must not become an empty envelope.
pub fn parse_update(body: &Value) -> ParsedUpdate {
    if body.get("entry").is_none() {
        return ParsedUpdate {
            messages: vec![message_from(body)],
            dropped: 0,
        };
    }
    let mut cloud = body
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
        .flatten();
    let messages: Vec<WhatsappMessage> = cloud
        .by_ref()
        .take(MAX_MESSAGES_PER_UPDATE)
        .map(message_from)
        .collect();
    let dropped = cloud.count();
    if dropped > 0 {
        crate::telemetry::log(
            crate::telemetry::Level::Warn,
            "whatsapp messages dropped at the provider edge",
            &[crate::telemetry::Field {
                key: "reason",
                value: "too many messages in one update",
            }],
        );
    }
    ParsedUpdate { messages, dropped }
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

    #[test]
    fn updates_without_messages_yield_nothing() {
        for b in [
            json!({"entry":[{"changes":[{"value":{"statuses":[{"id":"w","status":"delivered"}]}}]}]}),
            json!({"entry":[{"changes":[{"value":{"messages":"nope"}}]}]}),
            json!({"entry":[{"changes":[{"field":"messages"}]}]}),
            json!({"entry":[{"changes":[{"value":{"messages":[]}}]}]}),
            json!({"entry":[]}),
            json!({"entry":"x"}),
        ] {
            assert!(parse_messages(&b).is_empty(), "{b}");
        }
    }

    #[test]
    fn statuses_beside_messages_yield_only_the_messages() {
        let b = json!({"entry":[{"changes":[{"value":{
            "statuses":[{"id":"s","status":"read"}],
            "messages":[{"id":"m","from":"1","text":{"body":"yo"}}]}}]}]});
        let msgs = parse_messages(&b);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text, "yo");
    }

    #[test]
    fn at_most_the_cap_is_processed_and_the_rest_counted() {
        let many: Vec<Value> = (0..250)
            .map(|i| json!({"id": format!("w{i}"), "text": {"body": "x"}}))
            .collect();
        let b = json!({"entry":[{"changes":[{"value":{"messages": many}}]}]});
        let update = parse_update(&b);
        assert_eq!(update.messages.len(), MAX_MESSAGES_PER_UPDATE);
        assert_eq!(update.dropped, 250 - MAX_MESSAGES_PER_UPDATE);
        assert_eq!(update.messages[0].id.as_deref(), Some("w0"));
    }

    #[test]
    fn readable_wamids_stay_readable() {
        assert_eq!(
            whatsapp_envelope_id("wamid.HBgMNTU1MTIzNDU2Nzg5FQIAEhgUM0E=="),
            "whatsapp-wamid.HBgMNTU1MTIzNDU2Nzg5FQIAEhgUM0E=="
        );
        assert_eq!(whatsapp_envelope_id("  wamid.A \t"), "whatsapp-wamid.A");
    }

    #[test]
    fn hostile_wamids_are_hashed_deterministically() {
        let long = "a".repeat(10 * 1024);
        let exactly = "a".repeat(128);
        let over = "a".repeat(129);
        assert_eq!(
            whatsapp_envelope_id(&exactly),
            format!("whatsapp-{exactly}")
        );
        for hostile in [
            "a\nb",
            "a b",
            "a/b",
            "a\u{0}b",
            "wamid.\u{e9}",
            long.as_str(),
            over.as_str(),
        ] {
            let id = whatsapp_envelope_id(hostile);
            assert!(id.starts_with("whatsapp-~"), "{id}");
            assert_eq!(id.len(), "whatsapp-~".len() + 32, "{id}");
            assert!(
                id.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'~')
            );
            assert_eq!(id, whatsapp_envelope_id(hostile));
        }
        assert_ne!(whatsapp_envelope_id("a\nb"), whatsapp_envelope_id("a\nc"));
        assert_ne!(whatsapp_envelope_id("a b"), whatsapp_envelope_id("a/b"));
    }

    #[test]
    fn non_text_message_kinds_stay_empty() {
        for m in [
            json!({"id":"r","type":"reaction","reaction":{"message_id":"x","emoji":"x"}}),
            json!({"id":"b","type":"button","button":{"text":"Yes","payload":"p"}}),
            json!({"id":"i","type":"interactive","interactive":{"type":"button_reply","button_reply":{"id":"1","title":"T"}}}),
        ] {
            let b = json!({"entry":[{"changes":[{"value":{"messages":[m]}}]}]});
            let msgs = parse_messages(&b);
            assert_eq!(msgs.len(), 1);
            assert_eq!(msgs[0].text, "");
            assert!(msgs[0].pending.is_empty());
        }
    }
}
