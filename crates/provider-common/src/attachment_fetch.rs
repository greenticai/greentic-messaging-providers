//! Fetch references for inbound attachments (master plan contract C1).
//!
//! A provider never downloads and never puts a credential in the envelope. It
//! records, per attachment, HOW the host can obtain the bytes. The host
//! (`greentic-start`) resolves the reference, validates magic bytes and stores
//! the artifact.

use greentic_types::ChannelMessageEnvelope;
use greentic_types::messaging::Attachment;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const MAX_ATTACHMENTS: usize = 5;
pub const MAX_ATTACHMENT_BYTES: u64 = 10 * 1024 * 1024;
const MAX_NAME_LEN: usize = 120;
pub const FETCH_KEY: &str = "attachment_fetch";

const ALLOWED_MIME: &[&str] = &[
    "image/jpeg",
    "image/png",
    "image/gif",
    "image/webp",
    "text/plain",
    "text/markdown",
    "text/csv",
    "application/json",
    "application/pdf",
];

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FetchRef {
    /// Download `url` with `Authorization: Bearer <secret named secret_key>`.
    Bearer { url: String, secret_key: String },
    /// Host resolves `getFile` from this id; the token never leaves the host.
    TelegramFile { file_id: String },
    /// Host resolves the Graph media url, then downloads, both with the token.
    WhatsappMedia { media_id: String },
    /// `url` is already pre-authenticated and https.
    Public { url: String },
    /// Bytes are in `Attachment.content.data_base64` (WebChat upload only).
    Inline,
}

/// `Public` URLs are pre-authenticated credentials, so `Debug` never prints them.
impl std::fmt::Debug for FetchRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bearer { url, secret_key } => f
                .debug_struct("Bearer")
                .field("url", url)
                .field("secret_key", secret_key)
                .finish(),
            Self::TelegramFile { file_id } => f
                .debug_struct("TelegramFile")
                .field("file_id", file_id)
                .finish(),
            Self::WhatsappMedia { media_id } => f
                .debug_struct("WhatsappMedia")
                .field("media_id", media_id)
                .finish(),
            Self::Public { .. } => f
                .debug_struct("Public")
                .field("url", &"<redacted>")
                .finish(),
            Self::Inline => f.write_str("Inline"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PendingAttachment {
    pub mime_type: String,
    pub name: Option<String>,
    pub size_bytes: Option<u64>,
    pub fetch: FetchRef,
    pub inline_base64: Option<String>,
}

/// A secret NAME: slash allowed, charset `[A-Za-z0-9_./-]`, 1..=128 chars.
fn valid_secret_key(key: &str) -> bool {
    (1..=128).contains(&key.len())
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'/' | b'-'))
}

pub fn is_allowed_mime(mime: &str) -> bool {
    let base = mime
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    ALLOWED_MIME.contains(&base.as_str())
}

/// Strip directories and control characters; cap length. `None` when nothing is left.
pub fn sanitize_name(raw: &str) -> Option<String> {
    let last = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = last.chars().filter(|c| !c.is_control()).collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        return None;
    }
    Some(trimmed.chars().take(MAX_NAME_LEN).collect())
}

/// Cheap magic-byte sniff for the v1 types. The host re-validates; this is a
/// provider-edge guard only. SVG and HTML are never reported.
pub fn sniff_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if bytes.starts_with(b"\xff\xd8\xff") {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    if bytes.starts_with(b"%PDF-") {
        return Some("application/pdf");
    }
    let head = &bytes[..bytes.len().min(512)];
    let looks_text = !head.is_empty()
        && head.iter().all(|b| {
            *b == b'\n' || *b == b'\r' || *b == b'\t' || (0x20..0x7f).contains(b) || *b >= 0x80
        });
    if looks_text {
        let lower = String::from_utf8_lossy(head)
            .trim_start()
            .to_ascii_lowercase();
        if lower.starts_with('<') {
            return None;
        }
        return Some("text/plain");
    }
    None
}

/// Write pending attachments onto the envelope (`url` stays `None`), plus the
/// parallel `extensions["attachment_fetch"]` list. Applies the per-message cap
/// and the MIME/size pre-checks; anything rejected is counted in
/// `metadata["attachments_dropped"]`.
pub fn apply_fetch_refs(envelope: &mut ChannelMessageEnvelope, pending: Vec<PendingAttachment>) {
    let mut kept: Vec<(Attachment, Value)> = Vec::new();
    let mut dropped = 0usize;
    for item in pending {
        // Inline bytes are bounded by their own length, never by a declared size
        // that may be absent or wrong: base64 carries 3 bytes per 4 characters.
        let inline_bytes = item
            .inline_base64
            .as_ref()
            .map(|b64| (b64.len() as u64).div_ceil(4) * 3);
        let too_big = item.size_bytes.is_some_and(|s| s > MAX_ATTACHMENT_BYTES)
            || inline_bytes.is_some_and(|s| s > MAX_ATTACHMENT_BYTES);
        let bad_key = matches!(&item.fetch, FetchRef::Bearer { secret_key, .. } if !valid_secret_key(secret_key));
        if kept.len() >= MAX_ATTACHMENTS || too_big || bad_key || !is_allowed_mime(&item.mime_type)
        {
            dropped += 1;
            continue;
        }
        let content = item
            .inline_base64
            .as_ref()
            .map(|b64| json!({ "data_base64": b64 }));
        let attachment = Attachment {
            mime_type: item.mime_type,
            url: None,
            content,
            name: item.name.as_deref().and_then(sanitize_name),
            size_bytes: item.size_bytes,
        };
        let fetch = serde_json::to_value(&item.fetch).unwrap_or(Value::Null);
        kept.push((attachment, fetch));
    }
    if dropped > 0 {
        envelope
            .metadata
            .insert("attachments_dropped".to_string(), dropped.to_string());
    }
    if kept.is_empty() {
        return;
    }
    let refs: Vec<Value> = kept.iter().map(|(_, f)| f.clone()).collect();
    envelope.attachments = kept.into_iter().map(|(a, _)| a).collect();
    envelope
        .extensions
        .insert(FETCH_KEY.to_string(), Value::Array(refs));
}

#[cfg(test)]
mod tests {
    use super::*;
    use greentic_types::{ChannelMessageEnvelope, EnvId, TenantCtx, TenantId};
    use serde_json::json;

    fn empty_envelope() -> ChannelMessageEnvelope {
        ChannelMessageEnvelope {
            id: "t".into(),
            tenant: TenantCtx::new(
                EnvId::try_from("default").expect("env"),
                TenantId::try_from("default").expect("tenant"),
            ),
            channel: "c".into(),
            session_id: "s".into(),
            reply_scope: None,
            from: None,
            to: Vec::new(),
            correlation_id: None,
            text: None,
            attachments: Vec::new(),
            metadata: Default::default(),
            extensions: Default::default(),
        }
    }

    fn pending(n: usize) -> Vec<PendingAttachment> {
        (0..n)
            .map(|i| PendingAttachment {
                mime_type: "image/png".into(),
                name: Some(format!("f{i}.png")),
                size_bytes: Some(10),
                fetch: FetchRef::Public {
                    url: format!("https://x.test/{i}"),
                },
                inline_base64: None,
            })
            .collect()
    }

    #[test]
    fn writes_null_url_and_parallel_fetch_list() {
        let mut env = empty_envelope();
        apply_fetch_refs(&mut env, pending(2));
        assert_eq!(env.attachments.len(), 2);
        assert!(env.attachments.iter().all(|a| a.url.is_none()));
        let refs = env.extensions.get("attachment_fetch").expect("refs");
        assert_eq!(refs.as_array().expect("array").len(), 2);
        assert_eq!(
            refs[0],
            json!({"kind": "public", "url": "https://x.test/0"})
        );
    }

    #[test]
    fn caps_attachments_at_five() {
        let mut env = empty_envelope();
        apply_fetch_refs(&mut env, pending(6));
        assert_eq!(env.attachments.len(), 5);
        assert_eq!(
            env.metadata.get("attachments_dropped").map(String::as_str),
            Some("1")
        );
    }

    #[test]
    fn disallowed_mime_is_dropped_and_counted() {
        let mut env = empty_envelope();
        let mut p = pending(1);
        p[0].mime_type = "image/svg+xml".into();
        apply_fetch_refs(&mut env, p);
        assert!(env.attachments.is_empty());
        assert_eq!(
            env.metadata.get("attachments_dropped").map(String::as_str),
            Some("1")
        );
    }

    #[test]
    fn oversized_declared_size_is_dropped() {
        let mut env = empty_envelope();
        let mut p = pending(1);
        p[0].size_bytes = Some(MAX_ATTACHMENT_BYTES + 1);
        apply_fetch_refs(&mut env, p);
        assert!(env.attachments.is_empty());
    }

    #[test]
    fn oversized_inline_bytes_are_dropped_even_without_a_declared_size() {
        let mut env = empty_envelope();
        let big = "A".repeat(((MAX_ATTACHMENT_BYTES as usize) / 3 + 1) * 4);
        let p = vec![PendingAttachment {
            mime_type: "image/png".into(),
            name: Some("a.png".into()),
            size_bytes: None,
            fetch: FetchRef::Inline,
            inline_base64: Some(big),
        }];
        apply_fetch_refs(&mut env, p);
        assert!(env.attachments.is_empty());
        assert!(!env.extensions.contains_key("attachment_fetch"));
        assert_eq!(
            env.metadata.get("attachments_dropped").map(String::as_str),
            Some("1")
        );
    }

    #[test]
    fn inline_bytes_ride_in_content_not_url() {
        let mut env = empty_envelope();
        let p = vec![PendingAttachment {
            mime_type: "image/png".into(),
            name: Some("a.png".into()),
            size_bytes: Some(3),
            fetch: FetchRef::Inline,
            inline_base64: Some("AQID".into()),
        }];
        apply_fetch_refs(&mut env, p);
        assert_eq!(
            env.attachments[0].content,
            Some(json!({"data_base64": "AQID"}))
        );
        assert_eq!(
            env.extensions["attachment_fetch"][0],
            json!({"kind": "inline"})
        );
    }

    #[test]
    fn sanitize_name_strips_paths_and_controls() {
        assert_eq!(sanitize_name("../../etc/passwd").as_deref(), Some("passwd"));
        assert_eq!(sanitize_name("a\u{0}b\n.png").as_deref(), Some("ab.png"));
        assert_eq!(sanitize_name("   ").as_deref(), None);
        let long = "x".repeat(300);
        assert_eq!(sanitize_name(&long).map(|s| s.len()), Some(120));
    }

    #[test]
    fn sniff_recognises_v1_types_and_rejects_svg() {
        assert_eq!(sniff_mime(b"\x89PNG\r\n\x1a\nrest"), Some("image/png"));
        assert_eq!(sniff_mime(b"\xff\xd8\xffrest"), Some("image/jpeg"));
        assert_eq!(sniff_mime(b"GIF89a.."), Some("image/gif"));
        assert_eq!(
            sniff_mime(b"RIFF\x00\x00\x00\x00WEBPVP8 "),
            Some("image/webp")
        );
        assert_eq!(sniff_mime(b"%PDF-1.7"), Some("application/pdf"));
        assert_eq!(sniff_mime(b"hello, plain text\n"), Some("text/plain"));
        assert_eq!(sniff_mime(b"<svg xmlns='x'></svg>"), None);
        assert_eq!(sniff_mime(b"<!doctype html><p>x"), None);
        assert_eq!(sniff_mime(b"\x00\x01\x02binary"), None);
    }

    #[test]
    fn invalid_secret_key_is_dropped() {
        let mut env = empty_envelope();
        let mk = |k: &str| PendingAttachment {
            mime_type: "image/png".into(),
            name: None,
            size_bytes: None,
            fetch: FetchRef::Bearer {
                url: "https://files.slack.com/x".into(),
                secret_key: k.into(),
            },
            inline_base64: None,
        };
        apply_fetch_refs(
            &mut env,
            vec![
                mk("SLACK_BOT_TOKEN"),
                mk("a b"),
                mk(""),
                mk(&"K".repeat(129)),
                mk("slack/bot.token-1"),
            ],
        );
        assert_eq!(env.attachments.len(), 2);
        assert_eq!(
            env.metadata.get("attachments_dropped").map(String::as_str),
            Some("3")
        );
        assert_eq!(
            env.extensions["attachment_fetch"][0]["secret_key"],
            "SLACK_BOT_TOKEN"
        );
    }

    #[test]
    fn public_url_is_redacted_in_debug() {
        let r = FetchRef::Public {
            url: "https://x.test/secret-token".into(),
        };
        assert!(!format!("{r:?}").contains("secret-token"));
        let p = PendingAttachment {
            mime_type: "image/png".into(),
            name: None,
            size_bytes: None,
            fetch: r,
            inline_base64: None,
        };
        assert!(!format!("{p:?}").contains("secret-token"));
    }

    #[test]
    fn fetch_ref_round_trips_through_json() {
        let r: FetchRef =
            serde_json::from_value(json!({"kind":"telegram_file","file_id":"F1"})).expect("de");
        assert_eq!(
            r,
            FetchRef::TelegramFile {
                file_id: "F1".into()
            }
        );
        let r: FetchRef =
            serde_json::from_value(json!({"kind":"whatsapp_media","media_id":"M1"})).expect("de");
        assert_eq!(
            serde_json::to_value(&r).expect("ser"),
            json!({"kind":"whatsapp_media","media_id":"M1"})
        );
    }
}
