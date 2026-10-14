//! The Microsoft Entra authority the "Sign in with Microsoft" button talks to.
//!
//! Unset means `common`, which is what every deployment before this setting
//! used, so the URLs it yields are byte-identical to the old constants.

pub const DEFAULT_AUTHORITY: &str = "common";

const MAX_AUTHORITY_LEN: usize = 128;

/// A value that cannot be one path segment of the authority URL.
#[derive(Debug, PartialEq, Eq)]
pub struct InvalidAuthority;

/// The authorize and token endpoints for `authority`: `common`,
/// `organizations`, `consumers`, a tenant GUID or a verified tenant domain.
///
/// Only `[A-Za-z0-9.-]` is accepted, so a value can never add a path, query,
/// userinfo or a different host to the URL it is spliced into.
pub fn endpoints(authority: Option<&str>) -> Result<(String, String), InvalidAuthority> {
    let authority = authority
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_AUTHORITY);
    let allowed = authority
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    if authority.len() > MAX_AUTHORITY_LEN
        || !allowed
        || authority.starts_with('.')
        || authority.ends_with('.')
        || authority.contains("..")
    {
        return Err(InvalidAuthority);
    }
    let base = format!("https://login.microsoftonline.com/{authority}/oauth2/v2.0");
    Ok((format!("{base}/authorize"), format!("{base}/token")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_or_blank_is_the_historical_common_endpoint() {
        for input in [None, Some(""), Some("   ")] {
            let (auth, token) = endpoints(input).expect("default");
            assert_eq!(
                auth,
                "https://login.microsoftonline.com/common/oauth2/v2.0/authorize"
            );
            assert_eq!(
                token,
                "https://login.microsoftonline.com/common/oauth2/v2.0/token"
            );
        }
    }

    #[test]
    fn a_tenant_id_organizations_and_a_domain_are_accepted() {
        for value in [
            "organizations",
            "consumers",
            "72f988bf-86f1-41af-91ab-2d7cd011db47",
            "contoso.onmicrosoft.com",
        ] {
            let (auth, token) = endpoints(Some(value)).expect(value);
            assert_eq!(
                auth,
                format!("https://login.microsoftonline.com/{value}/oauth2/v2.0/authorize")
            );
            assert!(token.contains(&format!("/{value}/oauth2/v2.0/token")));
        }
    }

    #[test]
    fn surrounding_whitespace_is_trimmed() {
        let (auth, _) = endpoints(Some("  organizations\n")).expect("trimmed");
        assert!(auth.contains("/organizations/"));
    }

    #[test]
    fn a_value_that_could_reshape_the_url_is_refused() {
        for value in [
            "evil.com/x",
            "a/b",
            "a?b",
            "a#b",
            "user@evil.com",
            "a:443",
            "a b",
            "..",
            ".x",
            "x.",
            "a..b",
            "t\u{e9}nant",
        ] {
            assert_eq!(endpoints(Some(value)), Err(InvalidAuthority), "{value:?}");
        }
        assert_eq!(endpoints(Some(&"a".repeat(129))), Err(InvalidAuthority));
    }
}
