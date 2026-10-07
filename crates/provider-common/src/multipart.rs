//! Minimal `multipart/form-data` parser (no streaming, bounded by the caller).

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    pub name: String,
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub data: Vec<u8>,
}

pub fn boundary_from_content_type(ct: &str) -> Option<String> {
    let lower = ct.to_ascii_lowercase();
    if !lower.trim_start().starts_with("multipart/form-data") {
        return None;
    }
    let idx = lower.find("boundary=")?;
    let raw = &ct[idx + "boundary=".len()..];
    let value = raw.split(';').next()?.trim().trim_matches('"');
    if value.is_empty() || value.len() > 200 {
        return None;
    }
    Some(value.to_string())
}

pub fn parse(body: &[u8], boundary: &str) -> Result<Vec<Part>, String> {
    let delim = format!("--{boundary}").into_bytes();
    let mut parts = Vec::new();
    let mut pos = find(body, &delim, 0).ok_or("missing opening boundary")? + delim.len();
    loop {
        if body[pos..].starts_with(b"--") {
            return Ok(parts);
        }
        if !body[pos..].starts_with(b"\r\n") {
            return Err("malformed boundary line".into());
        }
        pos += 2;
        let header_end = find(body, b"\r\n\r\n", pos).ok_or("truncated part headers")?;
        let headers =
            std::str::from_utf8(&body[pos..header_end]).map_err(|_| "non-utf8 headers")?;
        let data_start = header_end + 4;
        let next = find(body, &[b"\r\n".as_slice(), &delim].concat(), data_start)
            .ok_or("truncated part body")?;
        let (mut name, mut filename, mut content_type) = (None, None, None);
        for line in headers.split("\r\n") {
            let lower = line.to_ascii_lowercase();
            if lower.starts_with("content-disposition:") {
                name = disposition_param(line, "name");
                filename = disposition_param(line, "filename");
            } else if lower.starts_with("content-type:") {
                content_type = Some(line["content-type:".len()..].trim().to_string());
            }
        }
        parts.push(Part {
            name: name.ok_or("part without a name")?,
            filename,
            content_type,
            data: body[data_start..next].to_vec(),
        });
        pos = next + 2 + delim.len();
    }
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
}
