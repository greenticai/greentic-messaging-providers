//! Fetch references for inbound attachments (master plan contract C1).
//!
//! A provider never downloads and never puts a credential in the envelope. It
//! records, per attachment, HOW the host can obtain the bytes. The host
//! (`greentic-start`) resolves the reference, validates magic bytes and stores
//! the artifact.

use crate::telemetry;
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

#[derive(Clone)]
pub struct PendingAttachment {
    pub mime_type: String,
    pub name: Option<String>,
    pub size_bytes: Option<u64>,
    pub fetch: FetchRef,
    pub inline_base64: Option<String>,
}

/// Inline bytes can be ~14 MB of base64, so `Debug` prints only their length.
impl std::fmt::Debug for PendingAttachment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingAttachment")
            .field("mime_type", &self.mime_type)
            .field("name", &self.name)
            .field("size_bytes", &self.size_bytes)
            .field("fetch", &self.fetch)
            .field(
                "inline_base64_len",
                &self.inline_base64.as_ref().map(String::len),
            )
            .finish()
    }
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

/// Unicode format (Cf) characters that can disguise a file name (bidi
/// overrides, zero-width characters, BOM, ...).
fn is_format_char(c: char) -> bool {
    matches!(c,
        '\u{00AD}' | '\u{0600}'..='\u{0605}' | '\u{061C}' | '\u{06DD}' | '\u{070F}'
        | '\u{180E}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}'
        | '\u{2060}'..='\u{206F}' | '\u{FEFF}' | '\u{FFF9}'..='\u{FFFB}'
        | '\u{E0001}' | '\u{E0020}'..='\u{E007F}')
}

/// Strip directories, control and format characters; cap length (then trim
/// again). `None` when nothing usable is left, or the name is `.` / `..`.
pub fn sanitize_name(raw: &str) -> Option<String> {
    let last = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = last
        .chars()
        .filter(|c| !c.is_control() && !is_format_char(*c))
        .collect();
    let truncated: String = cleaned.trim().chars().take(MAX_NAME_LEN).collect();
    let name = truncated.trim();
    if name.is_empty() || name == "." || name == ".." {
        return None;
    }
    Some(name.to_string())
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

/// Shape check for a fetch URL: `https://`, a non-empty plain host with an
/// optional numeric port, no userinfo, no whitespace/control/backslash. The
/// host (greentic-start) owns the SSRF allow-list; this only stops the bot
/// token from being referenced for a URL that is plainly not an https target.
fn https_url_problem(url: &str) -> Option<&'static str> {
    let Some(rest) = url
        .get(..8)
        .filter(|p| p.eq_ignore_ascii_case("https://"))
        .map(|_| &url[8..])
    else {
        return Some("url is not https");
    };
    if url
        .chars()
        .any(|c| c.is_control() || c.is_whitespace() || c == '\\')
    {
        return Some("url has forbidden characters");
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') {
        return Some("url has userinfo");
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (authority, None),
    };
    if host.is_empty() {
        return Some("url has an empty host");
    }
    if !host
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
    {
        return Some("url host is malformed");
    }
    if port.is_some_and(|p| p.is_empty() || p.len() > 5 || !p.bytes().all(|b| b.is_ascii_digit())) {
        return Some("url port is malformed");
    }
    None
}

/// Why an item must not reach the envelope; `None` when it is fine.
fn rejection(item: &PendingAttachment) -> Option<&'static str> {
    // Inline bytes are bounded by their own length, never by a declared size
    // that may be absent or wrong: base64 carries 3 bytes per 4 characters.
    let inline_bytes = item
        .inline_base64
        .as_ref()
        .map(|b64| (b64.len() as u64).div_ceil(4) * 3);
    if item.size_bytes.is_some_and(|s| s > MAX_ATTACHMENT_BYTES)
        || inline_bytes.is_some_and(|s| s > MAX_ATTACHMENT_BYTES)
    {
        return Some("attachment too large");
    }
    if !is_allowed_mime(&item.mime_type) {
        return Some("mime type not allowed");
    }
    match (&item.fetch, &item.inline_base64) {
        (FetchRef::Inline, None) => return Some("inline ref without bytes"),
        (FetchRef::Inline, Some(_)) => {}
        (_, Some(_)) => return Some("bytes supplied for a non-inline ref"),
        (_, None) => {}
    }
    match &item.fetch {
        FetchRef::Bearer { url, secret_key } => {
            if !valid_secret_key(secret_key) {
                return Some("invalid secret_key");
            }
            https_url_problem(url)
        }
        FetchRef::Public { url } => https_url_problem(url),
        _ => None,
    }
}

fn warn_dropped(channel: &str, reason: &str) {
    // Never the url (a `Public` url is a credential) and never any token value.
    telemetry::log(
        telemetry::Level::Warn,
        "attachment dropped at the provider edge",
        &[
            telemetry::Field {
                key: telemetry::field::PROVIDER,
                value: channel,
            },
            telemetry::Field {
                key: "reason",
                value: reason,
            },
        ],
    );
}

/// Write pending attachments onto the envelope (`url` stays `None`), plus the
/// parallel `extensions["attachment_fetch"]` list. Only items that pass the
/// per-message cap and the MIME, size, https-url and inline-consistency checks
/// are kept, so the two lists are index-parallel by construction. Rejected
/// items are added to `metadata["attachments_dropped"]` (the counter
/// accumulates across calls). When nothing is kept the envelope's
/// `attachments` and `attachment_fetch` are left untouched; otherwise they are
/// replaced.
pub fn apply_fetch_refs(envelope: &mut ChannelMessageEnvelope, pending: Vec<PendingAttachment>) {
    let mut kept: Vec<(Attachment, Value)> = Vec::new();
    let mut dropped = 0usize;
    for item in pending {
        let reason = if kept.len() >= MAX_ATTACHMENTS {
            Some("too many attachments")
        } else {
            rejection(&item)
        };
        if let Some(reason) = reason {
            warn_dropped(&envelope.channel, reason);
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
        let previous = envelope
            .metadata
            .get("attachments_dropped")
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(0);
        envelope.metadata.insert(
            "attachments_dropped".to_string(),
            (previous + dropped).to_string(),
        );
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

/// Same as [`apply_fetch_refs`] for components that build envelopes as raw
/// JSON. Replaces `attachments`, adds `extensions.attachment_fetch` and the
/// `metadata.attachments_dropped` counter exactly as the typed variant does;
/// when nothing is kept only the dropped counter can change.
pub fn apply_to_value(envelope: &mut Value, pending: Vec<PendingAttachment>) {
    let channel = envelope
        .get("channel")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let mut typed = ChannelMessageEnvelope {
        id: String::new(),
        tenant: greentic_types::TenantCtx::new(
            greentic_types::EnvId::try_from("default").expect("env"),
            greentic_types::TenantId::try_from("default").expect("tenant"),
        ),
        channel,
        session_id: String::new(),
        reply_scope: None,
        from: None,
        to: Vec::new(),
        correlation_id: None,
        text: None,
        attachments: Vec::new(),
        metadata: Default::default(),
        extensions: Default::default(),
    };
    apply_fetch_refs(&mut typed, pending);
    let Some(map) = envelope.as_object_mut() else {
        return;
    };
    if !typed.attachments.is_empty() {
        map.insert(
            "attachments".to_string(),
            serde_json::to_value(&typed.attachments).unwrap_or(Value::Array(Vec::new())),
        );
    }
    if let Some(refs) = typed.extensions.get(FETCH_KEY) {
        let ext = map
            .entry("extensions".to_string())
            .or_insert_with(|| json!({}));
        if let Some(ext_map) = ext.as_object_mut() {
            ext_map.insert(FETCH_KEY.to_string(), refs.clone());
        }
    }
    if let Some(dropped) = typed.metadata.get("attachments_dropped") {
        let meta = map
            .entry("metadata".to_string())
            .or_insert_with(|| json!({}));
        if let Some(meta_map) = meta.as_object_mut() {
            meta_map.insert("attachments_dropped".to_string(), json!(dropped));
        }
    }
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
    fn apply_to_value_matches_typed_output() {
        let mut v = json!({"id":"x","channel":"c","attachments":[],"metadata":{}});
        apply_to_value(&mut v, pending(1));
        assert_eq!(v["attachments"][0]["mime_type"], "image/png");
        assert!(v["attachments"][0]["url"].is_null());
        assert_eq!(v["extensions"]["attachment_fetch"][0]["kind"], "public");
    }

    #[test]
    fn apply_to_value_with_nothing_kept_only_counts_drops() {
        let mut v = json!({"id":"x","attachments":[],"metadata":{}});
        let bad = vec![PendingAttachment {
            mime_type: "image/svg+xml".into(),
            name: None,
            size_bytes: Some(1),
            fetch: FetchRef::Public {
                url: "https://x.test/a".into(),
            },
            inline_base64: None,
        }];
        apply_to_value(&mut v, bad);
        assert_eq!(v["attachments"], json!([]));
        assert!(v.get("extensions").is_none());
        assert_eq!(v["metadata"]["attachments_dropped"], "1");
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

    fn bearer(url: &str) -> PendingAttachment {
        PendingAttachment {
            mime_type: "image/png".into(),
            name: Some("a.png".into()),
            size_bytes: Some(1),
            fetch: FetchRef::Bearer {
                url: url.into(),
                secret_key: "SLACK_BOT_TOKEN".into(),
            },
            inline_base64: None,
        }
    }

    fn public(url: &str) -> PendingAttachment {
        PendingAttachment {
            fetch: FetchRef::Public { url: url.into() },
            ..bearer("")
        }
    }

    #[test]
    fn non_https_or_malformed_urls_are_dropped() {
        let bad = [
            "http://files.slack.com/x",
            "https://user:pw@files.slack.com/x",
            "https://user@files.slack.com/x",
            "https:///path",
            "https://",
            "https://:443/x",
            "not a url",
            "",
            "ftp://x.test/a",
            "https://a b.test/x",
            "https://x.test\\evil.test/x",
        ];
        for url in bad {
            for item in [bearer(url), public(url)] {
                let mut env = empty_envelope();
                apply_fetch_refs(&mut env, vec![item]);
                assert!(env.attachments.is_empty(), "kept {url:?}");
                assert_eq!(
                    env.metadata.get("attachments_dropped").map(String::as_str),
                    Some("1"),
                    "{url:?}"
                );
            }
        }
        let mut env = empty_envelope();
        apply_fetch_refs(
            &mut env,
            vec![
                bearer("https://files.slack.com/x?y=1"),
                public("HTTPS://x.test:8443/p#f"),
            ],
        );
        assert_eq!(env.attachments.len(), 2);
    }

    #[test]
    fn lists_stay_index_parallel_when_middle_items_are_dropped() {
        let mut env = empty_envelope();
        let mk = |i: usize| PendingAttachment {
            mime_type: "image/png".into(),
            name: Some(format!("n{i}.png")),
            size_bytes: Some(100 + i as u64),
            fetch: FetchRef::Bearer {
                url: format!("https://files.slack.com/{i}"),
                secret_key: "SLACK_BOT_TOKEN".into(),
            },
            inline_base64: None,
        };
        let mut bad_mime = mk(1);
        bad_mime.mime_type = "image/svg+xml".into();
        let mut bad_key = mk(3);
        bad_key.fetch = FetchRef::Bearer {
            url: "https://files.slack.com/3".into(),
            secret_key: "a b".into(),
        };
        let mut big = mk(5);
        big.size_bytes = Some(MAX_ATTACHMENT_BYTES + 1);
        let mut http = mk(7);
        http.fetch = FetchRef::Bearer {
            url: "http://files.slack.com/7".into(),
            secret_key: "K".into(),
        };
        apply_fetch_refs(
            &mut env,
            vec![
                mk(0),
                bad_mime,
                mk(2),
                bad_key,
                mk(4),
                big,
                mk(6),
                http,
                mk(8),
            ],
        );
        let refs = env.extensions["attachment_fetch"]
            .as_array()
            .expect("array")
            .clone();
        assert_eq!(env.attachments.len(), 5);
        assert_eq!(refs.len(), env.attachments.len());
        for (i, (a, r)) in env.attachments.iter().zip(refs.iter()).enumerate() {
            let n = i * 2;
            assert_eq!(a.name.as_deref(), Some(format!("n{n}.png").as_str()));
            assert_eq!(a.size_bytes, Some(100 + n as u64));
            assert_eq!(a.mime_type, "image/png");
            assert_eq!(r["url"], format!("https://files.slack.com/{n}"));
        }
        assert_eq!(env.metadata["attachments_dropped"], "4");
    }

    #[test]
    fn inline_consistency_is_enforced() {
        let mut env = empty_envelope();
        let mut no_bytes = pending(1);
        no_bytes[0].fetch = FetchRef::Inline;
        let mut stray_bytes = pending(1);
        stray_bytes[0].inline_base64 = Some("AQID".into());
        apply_fetch_refs(&mut env, no_bytes.into_iter().chain(stray_bytes).collect());
        assert!(env.attachments.is_empty());
        assert_eq!(env.metadata["attachments_dropped"], "2");
    }

    #[test]
    fn sanitize_name_strips_format_and_bidi_characters() {
        assert_eq!(
            sanitize_name("a\u{202E}gnp.exe").as_deref(),
            Some("agnp.exe")
        );
        for c in [
            '\u{200B}', '\u{200F}', '\u{2060}', '\u{2066}', '\u{2069}', '\u{FEFF}', '\u{202A}',
        ] {
            assert_eq!(
                sanitize_name(&format!("x{c}y")).as_deref(),
                Some("xy"),
                "{c:?}"
            );
        }
        assert_eq!(sanitize_name("\u{202E}").as_deref(), None);
        assert_eq!(sanitize_name("dir/..").as_deref(), None);
        assert_eq!(sanitize_name("dir/.").as_deref(), None);
        let spaced = format!("{} z", "x".repeat(119));
        assert_eq!(
            sanitize_name(&spaced).as_deref(),
            Some("x".repeat(119).as_str())
        );
    }

    #[test]
    fn pending_debug_never_prints_inline_bytes() {
        let p = PendingAttachment {
            mime_type: "image/png".into(),
            name: None,
            size_bytes: None,
            fetch: FetchRef::Inline,
            inline_base64: Some("SECRETBYTES".into()),
        };
        let dbg = format!("{p:?}");
        assert!(!dbg.contains("SECRETBYTES"), "{dbg}");
        assert!(dbg.contains("11"), "{dbg}");
    }

    #[test]
    fn dropped_counter_accumulates_and_empty_apply_leaves_lists_alone() {
        let mut env = empty_envelope();
        apply_fetch_refs(&mut env, pending(2));
        let mut svg = pending(1);
        svg[0].mime_type = "image/svg+xml".into();
        apply_fetch_refs(&mut env, svg.clone());
        assert_eq!(env.metadata["attachments_dropped"], "1");
        assert_eq!(
            env.attachments.len(),
            2,
            "kept nothing: attachments untouched"
        );
        assert_eq!(
            env.extensions["attachment_fetch"]
                .as_array()
                .expect("array")
                .len(),
            2
        );
        apply_fetch_refs(&mut env, svg);
        assert_eq!(env.metadata["attachments_dropped"], "2");
    }
}
