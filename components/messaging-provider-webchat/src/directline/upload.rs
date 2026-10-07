// Direct Line upload validation, shared by the route and the envelope stamp.

use base64::{Engine as _, engine::general_purpose};
use greentic_types::messaging::universal_dto::Header;
use provider_common::{attachment_fetch, multipart};
use serde_json::{Map, Value};

/// Files per upload.
pub const MAX_UPLOAD_FILES: usize = attachment_fetch::MAX_ATTACHMENTS;
/// Bytes per file.
pub const MAX_UPLOAD_FILE_BYTES: usize = attachment_fetch::MAX_ATTACHMENT_BYTES as usize;
/// WebChat: 15 MiB per message (all files of one message share one upload).
pub const MAX_UPLOAD_BODY_BYTES: usize = multipart::MAX_BODY_BYTES;
/// Activity part, sized for a Web Chat 4.18 thumbnail (a data: URL in its ignored `attachments`).
pub const MAX_ACTIVITY_PART_BYTES: usize = 256 * 1024;

/// Why an upload is refused. Every message is fixed text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadRejection {
    BadRequest(&'static str),
    TooLarge(&'static str),
    UnsupportedType,
}

impl UploadRejection {
    pub fn status(self) -> u16 {
        match self {
            Self::BadRequest(_) => 400,
            Self::TooLarge(_) => 413,
            Self::UnsupportedType => 415,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::BadRequest(_) => "bad_request",
            Self::TooLarge(_) => "payload_too_large",
            Self::UnsupportedType => "unsupported_media_type",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::BadRequest(m) | Self::TooLarge(m) => m,
            Self::UnsupportedType => "file type is not allowed",
        }
    }
}

/// One accepted file; `content_type` comes from the bytes, never the client.
#[derive(Clone, PartialEq, Eq)]
pub struct UploadedFile {
    pub content_type: &'static str,
    pub name: Option<String>,
    pub data: Vec<u8>,
}

impl std::fmt::Debug for UploadedFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UploadedFile")
            .field("content_type", &self.content_type)
            .field("name", &self.name)
            .field("len", &self.data.len())
            .finish()
    }
}

/// The sanitised activity fields plus the accepted files.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedUpload {
    pub activity: Map<String, Value>,
    pub files: Vec<UploadedFile>,
}

impl ValidatedUpload {
    /// Metadata-only attachment list stored in conversation state (no bytes).
    pub fn attachment_metadata(&self) -> Vec<Value> {
        self.files
            .iter()
            .map(|file| {
                let mut meta = Map::new();
                meta.insert("contentType".into(), Value::from(file.content_type));
                if let Some(name) = &file.name {
                    meta.insert("name".into(), Value::from(name.as_str()));
                }
                meta.insert("size".into(), Value::from(file.data.len()));
                meta.insert("_greentic_upload".into(), Value::Bool(true));
                Value::Object(meta)
            })
            .collect()
    }
}

/// Pure and deterministic, so the envelope stamp can re-run it on the same request.
pub fn parse_upload(
    headers: &[Header],
    body_b64: &str,
) -> Result<ValidatedUpload, UploadRejection> {
    let content_type = headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case("content-type"))
        .map(|h| h.value.as_str())
        .unwrap_or_default();
    let boundary = multipart::boundary_from_content_type(content_type).ok_or(
        UploadRejection::BadRequest("expected multipart/form-data with a boundary"),
    )?;
    // Bound the decoded size from the base64 length before allocating.
    if body_b64.len() / 4 * 3 > MAX_UPLOAD_BODY_BYTES {
        return Err(UploadRejection::TooLarge("upload too large"));
    }
    let body = general_purpose::STANDARD
        .decode(body_b64.trim())
        .map_err(|_| UploadRejection::BadRequest("invalid body encoding"))?;
    if body.len() > MAX_UPLOAD_BODY_BYTES {
        return Err(UploadRejection::TooLarge("upload too large"));
    }
    let parts = multipart::parse(&body, &boundary)
        .map_err(|_| UploadRejection::BadRequest("malformed multipart body"))?;

    let mut activity = None;
    let mut files = Vec::new();
    for part in parts {
        match part.name.as_str() {
            "activity" => {
                if activity.is_some() {
                    return Err(UploadRejection::BadRequest("duplicate activity part"));
                }
                activity = Some(sanitize_activity(&part.data)?);
            }
            "file" => {
                if files.len() >= MAX_UPLOAD_FILES {
                    return Err(UploadRejection::BadRequest("too many files"));
                }
                files.push(validate_file(part)?);
            }
            _ => return Err(UploadRejection::BadRequest("unexpected part")),
        }
    }
    if files.is_empty() {
        return Err(UploadRejection::BadRequest("no file part"));
    }
    Ok(ValidatedUpload {
        activity: activity.unwrap_or_default(),
        files,
    })
}

fn validate_file(part: multipart::Part) -> Result<UploadedFile, UploadRejection> {
    let declared = part.content_type.as_deref();
    if declared.is_some_and(|ct| ct.trim().to_ascii_lowercase().starts_with("multipart/")) {
        return Err(UploadRejection::BadRequest(
            "nested multipart is not allowed",
        ));
    }
    if part.data.len() > MAX_UPLOAD_FILE_BYTES {
        return Err(UploadRejection::TooLarge("file too large"));
    }
    let content_type =
        sniff_upload(&part.data, declared).ok_or(UploadRejection::UnsupportedType)?;
    Ok(UploadedFile {
        content_type,
        name: part
            .filename
            .as_deref()
            .and_then(attachment_fetch::sanitize_name),
        data: part.data,
    })
}

/// Keep only `text`, `locale`, `channelData` and `from.{id,name}`; `attachments` (thumbnails) is ignored.
fn sanitize_activity(data: &[u8]) -> Result<Map<String, Value>, UploadRejection> {
    if data.len() > MAX_ACTIVITY_PART_BYTES {
        return Err(UploadRejection::TooLarge("activity part too large"));
    }
    let Ok(Value::Object(raw)) = serde_json::from_slice::<Value>(data) else {
        return Err(UploadRejection::BadRequest(
            "activity part must be a JSON object",
        ));
    };
    let mut out = Map::new();
    for key in ["text", "locale"] {
        if let Some(Value::String(s)) = raw.get(key) {
            out.insert(key.to_string(), Value::String(s.clone()));
        }
    }
    if let Some(channel_data @ Value::Object(_)) = raw.get("channelData") {
        out.insert("channelData".into(), channel_data.clone());
    }
    if let Some(Value::Object(from)) = raw.get("from") {
        let mut kept = Map::new();
        for key in ["id", "name"] {
            if let Some(Value::String(s)) = from.get(key) {
                kept.insert(key.to_string(), Value::String(s.clone()));
            }
        }
        if !kept.is_empty() {
            out.insert("from".into(), Value::Object(kept));
        }
    }
    Ok(out)
}

/// Allow-listed type from the bytes; `declared` only picks a text subtype.
pub fn sniff_upload(data: &[u8], declared: Option<&str>) -> Option<&'static str> {
    if let Some(mime) = attachment_fetch::sniff_mime(data)
        && matches!(
            mime,
            "image/jpeg" | "image/png" | "image/gif" | "image/webp" | "application/pdf"
        )
    {
        return Some(mime);
    }
    let text = std::str::from_utf8(data).ok()?;
    let allowed_control = |c: char| matches!(c, '\t' | '\n' | '\r' | '\u{0c}');
    if text.is_empty() || text.chars().any(|c| c.is_control() && !allowed_control(c)) {
        return None;
    }
    if text
        .trim_start_matches(|c: char| c == '\u{feff}' || c.is_whitespace())
        .starts_with('<')
    {
        return None;
    }
    let declared = declared
        .and_then(|d| d.split(';').next())
        .map(|d| d.trim().to_ascii_lowercase());
    Some(match declared.as_deref() {
        Some("text/csv") => "text/csv",
        Some("text/markdown") => "text/markdown",
        Some("application/json") if serde_json::from_str::<serde::de::IgnoredAny>(text).is_ok() => {
            "application/json"
        }
        _ => "text/plain",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nDATA";

    fn ct(boundary: &str) -> Vec<Header> {
        vec![Header {
            name: "Content-Type".into(),
            value: format!("multipart/form-data; boundary={boundary}"),
        }]
    }

    fn file_part(name: &str, filename: &str, ct: &str, bytes: &[u8]) -> Vec<u8> {
        let mut b = format!(
            "--BB\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\nContent-Type: {ct}\r\n\r\n"
        )
        .into_bytes();
        b.extend_from_slice(bytes);
        b.extend_from_slice(b"\r\n");
        b
    }

    fn activity_part(json: &str) -> Vec<u8> {
        format!(
            "--BB\r\nContent-Disposition: form-data; name=\"activity\"\r\nContent-Type: application/json\r\n\r\n{json}\r\n"
        )
        .into_bytes()
    }

    fn body(parts: &[Vec<u8>]) -> String {
        let mut b: Vec<u8> = parts.concat();
        b.extend_from_slice(b"--BB--\r\n");
        general_purpose::STANDARD.encode(b)
    }

    fn reject(parts: &[Vec<u8>]) -> UploadRejection {
        parse_upload(&ct("BB"), &body(parts)).expect_err("rejected")
    }

    #[test]
    fn accepts_a_png_with_an_activity_and_uses_the_sniffed_type() {
        let up = parse_upload(
            &ct("BB"),
            &body(&[
                activity_part(r#"{"type":"message","text":"look"}"#),
                file_part("file", "p.png", "image/png", PNG),
            ]),
        )
        .expect("accepted");
        assert_eq!(up.files.len(), 1);
        assert_eq!(up.files[0].content_type, "image/png");
        assert_eq!(up.files[0].name.as_deref(), Some("p.png"));
        assert_eq!(up.files[0].data, PNG);
        assert_eq!(up.activity.get("text"), Some(&Value::from("look")));
    }

    #[test]
    fn the_client_mime_never_decides_the_type() {
        let jpeg = b"\xff\xd8\xff\xe0JFIF";
        let up = parse_upload(
            &ct("BB"),
            &body(&[file_part("file", "a.png", "image/png", jpeg)]),
        )
        .expect("jpeg is allowed");
        assert_eq!(up.files[0].content_type, "image/jpeg");
        let up = parse_upload(
            &ct("BB"),
            &body(&[file_part("file", "a.txt", "image/svg+xml", b"plain words")]),
        )
        .expect("text is allowed");
        assert_eq!(up.files[0].content_type, "text/plain");
    }

    #[test]
    fn svg_html_and_binary_junk_are_415_whatever_is_declared() {
        for bytes in [
            &b"<svg xmlns='x'></svg>"[..],
            b"\xef\xbb\xbf  <svg/>",
            b"\n<!DOCTYPE html><html></html>",
            b"<?xml version='1.0'?><svg/>",
            b"MZ\x90\x00\x03",
            b"text with a \x00 nul",
            b"\x1b[31mansi",
            b"\xff\xfe not utf8 \xc3",
        ] {
            for declared in ["image/png", "text/plain", "application/pdf"] {
                assert_eq!(
                    reject(&[file_part("file", "x", declared, bytes)]),
                    UploadRejection::UnsupportedType,
                    "{bytes:?} as {declared}"
                );
            }
        }
        assert_eq!(
            reject(&[file_part("file", "x", "image/png", b"")]),
            UploadRejection::UnsupportedType
        );
    }

    #[test]
    fn text_subtypes_follow_the_declared_family_only_when_the_bytes_are_text() {
        let cases = [
            ("text/csv", &b"a,b\n1,2\n"[..], "text/csv"),
            ("text/markdown", b"# Title\n", "text/markdown"),
            ("application/json", b"{\"a\":1}", "application/json"),
            ("application/json", b"{not json", "text/plain"),
            ("image/png", b"hello", "text/plain"),
            ("text/html", b"hello", "text/plain"),
        ];
        for (declared, bytes, expected) in cases {
            assert_eq!(
                sniff_upload(bytes, Some(declared)),
                Some(expected),
                "{declared}"
            );
        }
        assert_eq!(
            sniff_upload(b"%PDF-1.7", Some("text/plain")),
            Some("application/pdf")
        );
        assert_eq!(
            sniff_upload(b"RIFF\0\0\0\0WEBPVP8 ", None),
            Some("image/webp")
        );
        assert_eq!(sniff_upload(b"GIF89a....", None), Some("image/gif"));
    }

    #[test]
    fn per_file_cap_is_413_and_counted_before_encoding() {
        let mut big = PNG.to_vec();
        big.resize(MAX_UPLOAD_FILE_BYTES + 1, 0);
        assert_eq!(
            reject(&[file_part("file", "p.png", "image/png", &big)]),
            UploadRejection::TooLarge("file too large")
        );
        let mut at_cap = PNG.to_vec();
        at_cap.resize(MAX_UPLOAD_FILE_BYTES, 0);
        assert!(
            parse_upload(
                &ct("BB"),
                &body(&[file_part("file", "p.png", "image/png", &at_cap)])
            )
            .is_ok()
        );
    }

    #[test]
    fn total_body_cap_is_413_before_decoding() {
        // Not even valid base64: only a length check before decoding says 413.
        let oversized = "!".repeat(MAX_UPLOAD_BODY_BYTES / 3 * 4 + 8);
        assert_eq!(
            parse_upload(&ct("BB"), &oversized),
            Err(UploadRejection::TooLarge("upload too large"))
        );
        let oversized_b64 = "A".repeat(MAX_UPLOAD_BODY_BYTES / 3 * 4 + 8);
        assert_eq!(
            parse_upload(&ct("BB"), &oversized_b64),
            Err(UploadRejection::TooLarge("upload too large"))
        );
    }

    #[test]
    fn six_files_are_refused_and_five_are_accepted() {
        let six: Vec<Vec<u8>> = (0..6)
            .map(|i| file_part("file", &format!("{i}.png"), "image/png", PNG))
            .collect();
        assert_eq!(reject(&six), UploadRejection::BadRequest("too many files"));
        let up = parse_upload(&ct("BB"), &body(&six[..5])).expect("five");
        assert_eq!(up.files.len(), 5);
    }

    #[test]
    fn malformed_requests_are_400_with_fixed_messages() {
        assert_eq!(
            parse_upload(&[], &body(&[file_part("file", "p.png", "image/png", PNG)])),
            Err(UploadRejection::BadRequest(
                "expected multipart/form-data with a boundary"
            ))
        );
        let truncated = general_purpose::STANDARD.encode(b"--BB\r\nbroken");
        assert_eq!(
            parse_upload(&ct("BB"), &truncated),
            Err(UploadRejection::BadRequest("malformed multipart body"))
        );
        assert_eq!(
            parse_upload(&ct("BB"), "!!not base64!!"),
            Err(UploadRejection::BadRequest("invalid body encoding"))
        );
        assert_eq!(reject(&[]), UploadRejection::BadRequest("no file part"));
        assert_eq!(
            reject(&[activity_part("{}")]),
            UploadRejection::BadRequest("no file part")
        );
    }

    #[test]
    fn structure_rules_unknown_parts_duplicate_activity_nested_multipart() {
        assert_eq!(
            reject(&[file_part("other", "p.png", "image/png", PNG)]),
            UploadRejection::BadRequest("unexpected part")
        );
        assert_eq!(
            reject(&[
                activity_part("{}"),
                activity_part("{}"),
                file_part("file", "p.png", "image/png", PNG)
            ]),
            UploadRejection::BadRequest("duplicate activity part")
        );
        assert_eq!(
            reject(&[file_part(
                "file",
                "n",
                "multipart/mixed; boundary=CC",
                b"--CC\r\n\r\nx\r\n--CC--"
            )]),
            UploadRejection::BadRequest("nested multipart is not allowed")
        );
        assert_eq!(
            reject(&[
                activity_part("[1,2]"),
                file_part("file", "p.png", "image/png", PNG)
            ]),
            UploadRejection::BadRequest("activity part must be a JSON object")
        );
        let big_activity = format!(r#"{{"text":"{}"}}"#, "a".repeat(MAX_ACTIVITY_PART_BYTES));
        assert_eq!(
            reject(&[
                activity_part(&big_activity),
                file_part("file", "p.png", "image/png", PNG)
            ]),
            UploadRejection::TooLarge("activity part too large")
        );
    }

    #[test]
    fn the_activity_is_sanitised_to_a_plain_message() {
        let up = parse_upload(
            &ct("BB"),
            &body(&[
                activity_part(
                    r#"{"type":"event","text":"hi","locale":"de","value":{"x":1},"attachments":[{"contentUrl":"http://169.254.169.254/"}],"from":{"id":"alice","name":"A","role":"bot"},"channelData":{"k":1},"_upload_inline":[1]}"#,
                ),
                file_part("file", "p.png", "image/png", PNG),
            ]),
        )
        .expect("accepted");
        let keys: Vec<&str> = up.activity.keys().map(String::as_str).collect();
        assert_eq!(keys, ["channelData", "from", "locale", "text"]);
        assert_eq!(
            up.activity["from"],
            serde_json::json!({"id":"alice","name":"A"})
        );
    }

    #[test]
    fn names_are_sanitised_and_metadata_carries_no_bytes() {
        let up = parse_upload(
            &ct("BB"),
            &body(&[file_part(
                "file",
                "../../etc/\u{202e}gnp.exe",
                "image/png",
                PNG,
            )]),
        )
        .expect("accepted");
        assert_eq!(up.files[0].name.as_deref(), Some("gnp.exe"));
        let meta = up.attachment_metadata();
        assert_eq!(
            meta,
            vec![serde_json::json!({
                "contentType": "image/png",
                "name": "gnp.exe",
                "size": PNG.len(),
                "_greentic_upload": true,
            })]
        );
        let debug = format!("{up:?}");
        assert!(!debug.contains("DATA"), "{debug}");
    }

    #[test]
    fn rejection_messages_never_echo_client_input() {
        let marker = "ZZSECRETZZ";
        let r = reject(&[file_part(marker, marker, marker, marker.as_bytes())]);
        assert!(!r.message().contains(marker));
        let r = reject(&[file_part(
            "file",
            marker,
            "image/png",
            b"<svg>ZZSECRETZZ</svg>",
        )]);
        assert!(!r.message().contains(marker));
    }
}
