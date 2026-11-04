//! Operator-chosen origins that may frame the hosted chat page.
//!
//! The answer `allowed_embed_origins` is a comma- or whitespace-separated list
//! of bare origins. greentic-start reads it from the provider config and serves
//! the chat page with `Content-Security-Policy: frame-ancestors 'self'
//! <origins>` (and no `X-Frame-Options`). Unset keeps today's behaviour: the
//! page carries no framing header.
//!
//! The value ends up in a response header, so setup refuses anything that is
//! not a plain origin instead of trying to repair it.

/// More origins than this is almost certainly a mistake and bloats a header.
pub(crate) const MAX_ALLOWED_EMBED_ORIGINS: usize = 20;

/// Parse and canonicalise the answer. `Ok(None)` for a blank value, otherwise
/// the origins lower-cased, de-duplicated and joined with `,`.
pub(crate) fn normalise_allowed_embed_origins(raw: &str) -> Result<Option<String>, String> {
    let mut origins: Vec<String> = Vec::new();
    for entry in raw
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|part| !part.is_empty())
    {
        let origin = normalise_origin(entry).ok_or_else(|| {
            format!(
                "allowed_embed_origins: `{entry}` is not a valid origin; use https://host[:port] \
                 (http://localhost[:port] is allowed for development), with no path, wildcard or credentials"
            )
        })?;
        if !origins.contains(&origin) {
            origins.push(origin);
        }
    }
    if origins.len() > MAX_ALLOWED_EMBED_ORIGINS {
        return Err(format!(
            "allowed_embed_origins: at most {MAX_ALLOWED_EMBED_ORIGINS} origins are allowed"
        ));
    }
    Ok((!origins.is_empty()).then(|| origins.join(",")))
}

fn normalise_origin(input: &str) -> Option<String> {
    let lowered = input.to_ascii_lowercase();
    let (scheme, rest) = if let Some(rest) = lowered.strip_prefix("https://") {
        ("https", rest)
    } else if let Some(rest) = lowered.strip_prefix("http://") {
        ("http", rest)
    } else {
        return None;
    };
    let (host, port) = match rest.split_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (rest, None),
    };
    if let Some(port) = port {
        let digits_only =
            !port.is_empty() && port.len() <= 5 && port.bytes().all(|b| b.is_ascii_digit());
        let value: u32 = port.parse().ok().filter(|_| digits_only)?;
        if value == 0 || value > 65535 {
            return None;
        }
    }
    if scheme == "http" && host != "localhost" {
        return None;
    }
    if !valid_dns_name(host) {
        return None;
    }
    Some(match port {
        Some(port) => format!("{scheme}://{host}:{port}"),
        None => format!("{scheme}://{host}"),
    })
}

fn valid_dns_name(host: &str) -> bool {
    if host.is_empty() || host.len() > 253 {
        return false;
    }
    let labels: Vec<&str> = host.split('.').collect();
    // An all-numeric last label would make this an IPv4 literal.
    if labels
        .last()
        .is_some_and(|label| label.bytes().all(|b| b.is_ascii_digit()))
    {
        return false;
    }
    labels.iter().all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_is_not_configured() {
        assert_eq!(normalise_allowed_embed_origins(""), Ok(None));
        assert_eq!(normalise_allowed_embed_origins(" , \n "), Ok(None));
    }

    #[test]
    fn origins_are_lowercased_deduplicated_and_joined() {
        assert_eq!(
            normalise_allowed_embed_origins(
                "https://Intranet.Example.com, https://intranet.example.com\nhttps://a.example.com:8443"
            ),
            Ok(Some(
                "https://intranet.example.com,https://a.example.com:8443".to_string()
            ))
        );
    }

    #[test]
    fn http_is_only_for_localhost() {
        assert_eq!(
            normalise_allowed_embed_origins("http://localhost:3000"),
            Ok(Some("http://localhost:3000".to_string()))
        );
        assert!(normalise_allowed_embed_origins("http://intranet.example.com").is_err());
    }

    #[test]
    fn anything_that_is_not_a_bare_origin_is_refused() {
        for bad in [
            "*",
            "https://*.example.com",
            "https://example.com/",
            "https://example.com/path",
            "https://example.com?x=1",
            "https://example.com#x",
            "https://user@example.com",
            "example.com",
            "ftp://example.com",
            "https://",
            "https://example.com:0",
            "https://example.com:65536",
            "https://example.com:80a",
            "https://127.0.0.1",
            "https://[::1]",
            "https://-a.example.com",
            "https://a..example.com",
            "https://example.com;script-src",
            "https://example.com'",
        ] {
            assert!(
                normalise_allowed_embed_origins(bad).is_err(),
                "{bad} must be refused"
            );
        }
    }

    #[test]
    fn one_bad_entry_refuses_the_whole_answer() {
        assert!(normalise_allowed_embed_origins("https://ok.example.com, *").is_err());
    }

    #[test]
    fn too_many_origins_are_refused() {
        let many = (0..=MAX_ALLOWED_EMBED_ORIGINS)
            .map(|i| format!("https://h{i}.example.com"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(normalise_allowed_embed_origins(&many).is_err());
    }
}
