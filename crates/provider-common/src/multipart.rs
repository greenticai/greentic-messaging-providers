//! Minimal `multipart/form-data` parser. No streaming; it is fed untrusted
//! bytes (WebChat uploads), so every dimension is capped.
//!
//! Only CRLF line endings are accepted (RFC 7578); a bare-LF body is an error.

/// Largest body `parse` will look at.
pub const MAX_BODY_BYTES: usize = 15 * 1024 * 1024;
/// Most parts `parse` returns before refusing.
pub const MAX_PARTS: usize = 16;
/// Largest header block of one part.
pub const MAX_HEADER_BYTES: usize = 8 * 1024;
/// RFC 2046 boundary length limit.
const MAX_BOUNDARY_LEN: usize = 70;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    pub name: String,
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub data: Vec<u8>,
}

/// Extract `boundary` from a `multipart/form-data` content type. Parameters are
/// split on `;` outside quotes, so a quoted boundary may contain `;` and a
/// `boundary=` inside another parameter's value is not mistaken for it.
pub fn boundary_from_content_type(ct: &str) -> Option<String> {
    let mut segments = split_params(ct).into_iter();
    if !segments
        .next()?
        .trim()
        .eq_ignore_ascii_case("multipart/form-data")
    {
        return None;
    }
    for seg in segments {
        let Some((key, value)) = seg.split_once('=') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("boundary") {
            let value = value.trim();
            let value = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .unwrap_or(value);
            if value.is_empty() || value.len() > MAX_BOUNDARY_LEN {
                return None;
            }
            return Some(value.to_string());
        }
    }
    None
}

fn split_params(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut quoted) = (0, false);
    for (i, c) in s.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ';' if !quoted => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

pub fn parse(body: &[u8], boundary: &str) -> Result<Vec<Part>, String> {
    if body.len() > MAX_BODY_BYTES {
        return Err("body too large".into());
    }
    if boundary.is_empty() || boundary.len() > MAX_BOUNDARY_LEN {
        return Err("invalid boundary".into());
    }
    let delim = format!("--{boundary}").into_bytes();
    // Every boundary after the first is preceded by CRLF.
    let crlf_delim = [b"\r\n".as_slice(), &delim].concat();
    let mut parts = Vec::new();
    let mut pos = find(body, &delim, 0).ok_or("missing opening boundary")? + delim.len();
    loop {
        if body[pos..].starts_with(b"--") {
            return Ok(parts);
        }
        if !body[pos..].starts_with(b"\r\n") {
            return Err("malformed boundary line".into());
        }
        if parts.len() >= MAX_PARTS {
            return Err("too many parts".into());
        }
        pos += 2;
        // The header block ends within MAX_HEADER_BYTES and before any boundary.
        let window_end = body.len().min(pos + MAX_HEADER_BYTES + 4);
        let header_end = find(&body[..window_end], b"\r\n\r\n", pos);
        let next_boundary = find_boundary(body, &crlf_delim, pos - 2);
        let header_end = match (header_end, next_boundary) {
            (Some(h), Some(b)) if h < b => h,
            (Some(h), None) => h,
            _ => return Err("missing or oversized part headers".into()),
        };
        if header_end - pos > MAX_HEADER_BYTES {
            return Err("part headers too large".into());
        }
        let headers =
            std::str::from_utf8(&body[pos..header_end]).map_err(|_| "non-utf8 headers")?;
        let data_start = header_end + 4;
        let next = find_boundary(body, &crlf_delim, data_start).ok_or("truncated part body")?;
        let (mut name, mut filename, mut content_type) = (None, None, None);
        let mut seen_disposition = false;
        for line in headers.split("\r\n") {
            let lower = line.to_ascii_lowercase();
            // First Content-Disposition wins; later ones are ignored.
            if lower.starts_with("content-disposition:") && !seen_disposition {
                seen_disposition = true;
                name = disposition_param(line, "name");
                filename = disposition_param(line, "filename").filter(|f| !f.is_empty());
            } else if lower.starts_with("content-type:") && content_type.is_none() {
                content_type = Some(line["content-type:".len()..].trim().to_string());
            }
        }
        parts.push(Part {
            name: name.ok_or("part without a name")?,
            filename,
            content_type,
            data: body[data_start..next].to_vec(),
        });
        pos = next + crlf_delim.len();
    }
}

/// Find `CRLF--boundary` at or after `from` that is a real boundary line, i.e.
/// followed by `--` or CRLF (so `--boundaryfoo` inside content is data).
fn find_boundary(body: &[u8], crlf_delim: &[u8], mut from: usize) -> Option<usize> {
    while let Some(at) = find(body, crlf_delim, from) {
        let after = &body[at + crlf_delim.len()..];
        if after.starts_with(b"--") || after.starts_with(b"\r\n") {
            return Some(at);
        }
        from = at + 1;
    }
    None
}

fn disposition_param(line: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=\"");
    // `name="` must not match inside `filename="`.
    let mut search_from = 0;
    while let Some(rel) = line[search_from..].find(&needle) {
        let at = search_from + rel;
        let preceded_ok = at == 0 || matches!(line.as_bytes()[at - 1], b' ' | b';');
        if preceded_ok {
            let start = at + needle.len();
            let end = line[start..].find('"')? + start;
            return Some(line[start..end].to_string());
        }
        search_from = at + needle.len();
    }
    None
}

fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from > haystack.len() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|i| i + from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(b"--XX\r\nContent-Disposition: form-data; name=\"activity\"\r\nContent-Type: application/json\r\n\r\n{\"type\":\"message\"}\r\n");
        b.extend_from_slice(b"--XX\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.png\"\r\nContent-Type: image/png\r\n\r\n");
        b.extend_from_slice(b"\x89PNG\r\n\x1a\nDATA\r\n--XX--\r\n");
        b
    }

    #[test]
    fn parses_activity_and_file_parts() {
        let parts = parse(&body(), "XX").expect("parts");
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].name, "activity");
        assert_eq!(parts[0].data, b"{\"type\":\"message\"}");
        assert_eq!(parts[1].name, "file");
        assert_eq!(parts[1].filename.as_deref(), Some("a.png"));
        assert_eq!(parts[1].content_type.as_deref(), Some("image/png"));
        assert!(parts[1].data.starts_with(b"\x89PNG"));
    }

    #[test]
    fn name_does_not_match_inside_filename() {
        let parts = parse(&body(), "XX").expect("parts");
        assert_eq!(parts[1].name, "file");
    }

    #[test]
    fn truncated_body_is_an_error_not_a_panic() {
        let b = body();
        assert!(parse(&b[..b.len() - 20], "XX").is_err());
        assert!(parse(b"", "XX").is_err());
        assert!(
            parse(
                b"--XX\r\nContent-Disposition: form-data; name=\"f\"\r\n",
                "XX"
            )
            .is_err()
        );
    }

    #[test]
    fn boundary_extraction() {
        assert_eq!(
            boundary_from_content_type("multipart/form-data; boundary=\"abc123\"").as_deref(),
            Some("abc123")
        );
        assert_eq!(boundary_from_content_type("application/json"), None);
        assert_eq!(boundary_from_content_type("multipart/form-data"), None);
    }

    fn part(name: &str, data: &str) -> String {
        format!("--XX\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{data}\r\n")
    }

    #[test]
    fn unterminated_header_block_cannot_swallow_later_parts() {
        let body = format!(
            "--XX\r\nContent-Disposition: form-data; name=\"a\"\r\n{}--XX--\r\n",
            part("b", "v")
        );
        assert!(parse(body.as_bytes(), "XX").is_err());
    }

    #[test]
    fn over_total_cap_is_refused() {
        let mut body = part("a", "v").into_bytes();
        body.resize(MAX_BODY_BYTES + 1, b'x');
        assert!(parse(&body, "XX").unwrap_err().contains("too large"));
    }

    #[test]
    fn over_part_count_is_refused() {
        let mut body = String::new();
        for i in 0..=MAX_PARTS {
            body.push_str(&part(&format!("p{i}"), "v"));
        }
        body.push_str("--XX--\r\n");
        assert!(
            parse(body.as_bytes(), "XX")
                .unwrap_err()
                .contains("too many parts")
        );
        let ok = body.replacen(&part(&format!("p{MAX_PARTS}"), "v"), "", 1);
        assert_eq!(parse(ok.as_bytes(), "XX").expect("at cap").len(), MAX_PARTS);
    }

    #[test]
    fn over_header_size_is_refused() {
        let body = format!(
            "--XX\r\nContent-Disposition: form-data; name=\"a\"\r\nX-Pad: {}\r\n\r\nv\r\n--XX--\r\n",
            "p".repeat(MAX_HEADER_BYTES)
        );
        assert!(
            parse(body.as_bytes(), "XX")
                .unwrap_err()
                .contains("headers")
        );
    }

    #[test]
    fn boundary_text_inside_content_is_data_not_a_boundary() {
        let body = part("a", "line --XX not a boundary\r\n--XXfoo still data");
        let body = format!("{body}--XX--\r\n");
        let parts = parse(body.as_bytes(), "XX").expect("parts");
        assert_eq!(parts.len(), 1);
        assert_eq!(
            parts[0].data,
            b"line --XX not a boundary\r\n--XXfoo still data"
        );
    }

    #[test]
    fn empty_filename_is_none() {
        let body = "--XX\r\nContent-Disposition: form-data; name=\"f\"; filename=\"\"\r\n\r\n\r\n--XX--\r\n";
        let parts = parse(body.as_bytes(), "XX").expect("parts");
        assert_eq!(parts[0].filename, None);
        assert!(parts[0].data.is_empty());
    }

    #[test]
    fn duplicate_content_disposition_first_wins() {
        let body = "--XX\r\nContent-Disposition: form-data; name=\"good\"\r\nContent-Disposition: form-data\r\n\r\nv\r\n--XX--\r\n";
        let parts = parse(body.as_bytes(), "XX").expect("parts");
        assert_eq!(parts[0].name, "good");
    }

    #[test]
    fn quoted_boundary_may_contain_semicolon_and_params_do_not_confuse() {
        assert_eq!(
            boundary_from_content_type("multipart/form-data; boundary=\"a;b\"; charset=x")
                .as_deref(),
            Some("a;b")
        );
        assert_eq!(
            boundary_from_content_type("multipart/form-data; foo=\"boundary=evil\"; boundary=real")
                .as_deref(),
            Some("real")
        );
        assert_eq!(
            boundary_from_content_type("multipart/form-data; foo=\"boundary=evil\""),
            None
        );
        assert!(
            boundary_from_content_type(&format!(
                "multipart/form-data; boundary={}",
                "b".repeat(70)
            ))
            .is_some()
        );
        assert!(
            boundary_from_content_type(&format!(
                "multipart/form-data; boundary={}",
                "b".repeat(71)
            ))
            .is_none()
        );
    }

    #[test]
    fn bare_lf_bodies_are_rejected() {
        let body = "--XX\nContent-Disposition: form-data; name=\"a\"\n\nv\n--XX--\n";
        assert!(parse(body.as_bytes(), "XX").is_err());
    }

    #[test]
    fn overlong_boundary_is_refused_by_parse() {
        assert!(parse(b"--x", &"b".repeat(71)).is_err());
    }
}
