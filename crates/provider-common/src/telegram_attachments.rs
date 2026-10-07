//! Telegram `message` -> fetch references (master plan contract C1).
//!
//! Shared by the provider `ingest_http` and the legacy `messaging-ingress-telegram`
//! so the two ingest paths cannot drift.
//!
//! The reference is `{kind: "telegram_file", file_id}` and never a URL: the
//! Telegram download URL embeds the bot token
//! (`api.telegram.org/file/bot<TOKEN>/<path>`), so the provider never builds,
//! logs or stores one. The host resolves `getFile` with its own token.
//!
//! Albums (`media_group_id`) arrive as separate updates, one per item. They are
//! deliberately NOT merged here: each update yields its own envelope.
//!
//! Assumed Bot API shapes (not verified against this repo): `photo` is an array
//! of `{file_id, file_unique_id, width, height, file_size?}` in any order;
//! photos are re-encoded as JPEG and carry no mime type; `document` is
//! `{file_id, file_name?, mime_type?, file_size?}`; `caption` is the text of a
//! media message.

use crate::attachment_fetch::{FetchRef, PendingAttachment};
use serde_json::Value;

fn file_id(v: &Value) -> Option<String> {
    v.get("file_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// The photo with the most pixels (ties: larger `file_size`, then later in the
/// array). Array order is not trusted.
fn largest_photo(photos: &[Value]) -> Option<&Value> {
    photos
        .iter()
        .filter(|p| file_id(p).is_some())
        .max_by_key(|p| {
            let dim = |k: &str| p.get(k).and_then(Value::as_u64).unwrap_or(0);
            (
                dim("width").saturating_mul(dim("height")),
                p.get("file_size").and_then(Value::as_u64).unwrap_or(0),
            )
        })
}

/// `photo` (largest size only) and `document` of a Telegram message.
pub fn telegram_pending_attachments(message: &Value) -> Vec<PendingAttachment> {
    let mut out = Vec::new();
    if let Some(best) = message
        .get("photo")
        .and_then(Value::as_array)
        .and_then(|photos| largest_photo(photos))
        && let Some(id) = file_id(best)
    {
        out.push(PendingAttachment {
            mime_type: "image/jpeg".to_string(),
            name: None,
            size_bytes: best.get("file_size").and_then(Value::as_u64),
            fetch: FetchRef::TelegramFile { file_id: id },
            inline_base64: None,
        });
    }
    if let Some(doc) = message.get("document")
        && let Some(id) = file_id(doc)
    {
        out.push(PendingAttachment {
            mime_type: doc
                .get("mime_type")
                .and_then(Value::as_str)
                .unwrap_or("application/octet-stream")
                .to_string(),
            name: doc
                .get("file_name")
                .and_then(Value::as_str)
                .map(str::to_string),
            size_bytes: doc.get("file_size").and_then(Value::as_u64),
            fetch: FetchRef::TelegramFile { file_id: id },
            inline_base64: None,
        });
    }
    out
}

/// Text of a message: `text`, else the media `caption`, else empty.
pub fn telegram_message_text(message: &Value) -> &str {
    message
        .get("text")
        .and_then(Value::as_str)
        .or_else(|| message.get("caption").and_then(Value::as_str))
        .unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// (name, message, expected `(mime, name, size, file_id)` list)
    #[allow(clippy::type_complexity)]
    fn cases() -> Vec<(
        &'static str,
        Value,
        Vec<(
            &'static str,
            Option<&'static str>,
            Option<u64>,
            &'static str,
        )>,
    )> {
        vec![
            (
                "largest by pixels, array order untrusted",
                json!({"photo":[
                    {"file_id":"BIG","width":1280,"height":853,"file_size":98000},
                    {"file_id":"SMALL","width":90,"height":60,"file_size":1200}]}),
                vec![("image/jpeg", None, Some(98000), "BIG")],
            ),
            (
                "pixels beat a larger file_size",
                json!({"photo":[
                    {"file_id":"A","width":100,"height":100,"file_size":999999},
                    {"file_id":"B","width":200,"height":200,"file_size":10}]}),
                vec![("image/jpeg", None, Some(10), "B")],
            ),
            (
                "no dimensions falls back to file_size",
                json!({"photo":[{"file_id":"S","file_size":1},{"file_id":"L","file_size":9}]}),
                vec![("image/jpeg", None, Some(9), "L")],
            ),
            (
                "document keeps name mime size",
                json!({"document":{"file_id":"D1","file_name":"n.csv","mime_type":"text/csv","file_size":50}}),
                vec![("text/csv", Some("n.csv"), Some(50), "D1")],
            ),
            (
                "document without mime",
                json!({"document":{"file_id":"D2"}}),
                vec![("application/octet-stream", None, None, "D2")],
            ),
            (
                "entries without file_id are ignored",
                json!({"photo":[{"width":5,"height":5}],"document":{"file_name":"x"}}),
                vec![],
            ),
            ("no media", json!({"text":"hi"}), vec![]),
        ]
    }

    #[test]
    fn table() {
        for (name, message, want) in cases() {
            let got: Vec<_> = telegram_pending_attachments(&message)
                .into_iter()
                .map(|p| {
                    let FetchRef::TelegramFile { file_id } = p.fetch else {
                        panic!("{name}: wrong ref kind");
                    };
                    (p.mime_type, p.name, p.size_bytes, file_id)
                })
                .collect();
            let want: Vec<_> = want
                .into_iter()
                .map(|(m, n, s, f)| (m.to_string(), n.map(str::to_string), s, f.to_string()))
                .collect();
            assert_eq!(got, want, "{name}");
        }
    }

    #[test]
    fn text_prefers_text_then_caption() {
        assert_eq!(
            telegram_message_text(&json!({"text":"hi","caption":"c"})),
            "hi"
        );
        assert_eq!(telegram_message_text(&json!({"caption":"c"})), "c");
        assert_eq!(telegram_message_text(&json!({})), "");
    }

    #[test]
    fn no_url_with_the_bot_token_is_ever_produced() {
        let p = telegram_pending_attachments(&json!({"photo":[{"file_id":"X","file_size":1}]}));
        let dump = format!("{p:?}");
        assert!(!dump.contains("api.telegram.org") && !dump.contains("/bot"));
    }
}
