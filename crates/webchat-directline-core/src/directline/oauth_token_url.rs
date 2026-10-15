//! Which token endpoint a stored `client_secret` may be sent to.
//!
//! `POST /oauth/token-exchange` used to POST to whatever `token_url` the
//! request body named, with the provider's stored `client_secret` attached, so
//! any caller could aim that secret at a host of its choosing. The endpoint is
//! now derived from the operator's own configuration, and a `token_url` in the
//! request may only repeat it.

use serde_json::Value;

use super::microsoft_authority;

/// Why no token endpoint can be named for the request.
#[derive(Debug, PartialEq, Eq)]
pub enum TokenUrlError {
    /// No configured provider has this id (or its endpoint is not configured).
    UnknownProvider,
    /// The configured endpoint is not an `https://` URL.
    NotHttps,
    /// The request named a `token_url` other than the configured one.
    Mismatch,
}

/// The one token endpoint `provider_id` may use, from configuration only.
///
/// `read` returns a configured value (blank is `None`), exactly as the
/// provider's secret reader does. `requested` is the `token_url` the caller
/// sent: absent or blank is fine, anything else must equal the configured URL.
pub fn resolve(
    provider_id: &str,
    requested: Option<&str>,
    read: &dyn Fn(&str) -> Option<String>,
) -> Result<String, TokenUrlError> {
    let expected = match provider_id {
        "google" => "https://oauth2.googleapis.com/token".to_string(),
        "github" => "https://github.com/login/oauth/access_token".to_string(),
        "microsoft" => {
            microsoft_authority::endpoints(read("oauth_microsoft_authority").as_deref())
                .map_err(|_| TokenUrlError::UnknownProvider)?
                .1
        }
        "custom" => read("oauth_custom_token_url").ok_or(TokenUrlError::UnknownProvider)?,
        other => read("oauth_providers")
            .and_then(|json| serde_json::from_str::<Vec<Value>>(&json).ok())
            .and_then(|list| {
                list.iter()
                    .find(|p| p["id"].as_str() == Some(other))
                    .and_then(|p| p["token_url"].as_str().map(str::to_string))
            })
            .filter(|url| !url.trim().is_empty())
            .ok_or(TokenUrlError::UnknownProvider)?,
    };
    let expected = expected.trim().to_string();
    if !expected.starts_with("https://") {
        return Err(TokenUrlError::NotHttps);
    }
    match requested.map(str::trim).filter(|r| !r.is_empty()) {
        Some(r) if r != expected => Err(TokenUrlError::Mismatch),
        _ => Ok(expected),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn reader(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| map.get(k).cloned()
    }

    #[test]
    fn built_in_providers_resolve_without_configuration() {
        let r = reader(&[]);
        assert_eq!(
            resolve("google", None, &r).unwrap(),
            "https://oauth2.googleapis.com/token"
        );
        assert_eq!(
            resolve("github", Some(""), &r).unwrap(),
            "https://github.com/login/oauth/access_token"
        );
    }

    #[test]
    fn microsoft_follows_the_configured_authority_and_defaults_to_common() {
        assert_eq!(
            resolve("microsoft", None, &reader(&[])).unwrap(),
            "https://login.microsoftonline.com/common/oauth2/v2.0/token"
        );
        let r = reader(&[("oauth_microsoft_authority", "contoso.onmicrosoft.com")]);
        assert_eq!(
            resolve("microsoft", None, &r).unwrap(),
            "https://login.microsoftonline.com/contoso.onmicrosoft.com/oauth2/v2.0/token"
        );
    }

    #[test]
    fn a_request_token_url_may_only_repeat_the_configured_one() {
        let r = reader(&[]);
        assert_eq!(
            resolve("google", Some("https://oauth2.googleapis.com/token"), &r).unwrap(),
            "https://oauth2.googleapis.com/token"
        );
        assert_eq!(
            resolve("google", Some("https://evil.example/token"), &r),
            Err(TokenUrlError::Mismatch)
        );
    }

    #[test]
    fn custom_uses_the_configured_url_and_needs_https() {
        let ok = reader(&[("oauth_custom_token_url", "https://idp.example/oauth/token")]);
        assert_eq!(
            resolve("custom", None, &ok).unwrap(),
            "https://idp.example/oauth/token"
        );
        assert_eq!(
            resolve("custom", Some("https://evil.example/t"), &ok),
            Err(TokenUrlError::Mismatch)
        );
        let http = reader(&[("oauth_custom_token_url", "http://idp.example/token")]);
        assert_eq!(resolve("custom", None, &http), Err(TokenUrlError::NotHttps));
        assert_eq!(
            resolve("custom", None, &reader(&[])),
            Err(TokenUrlError::UnknownProvider)
        );
    }

    #[test]
    fn composed_providers_resolve_by_id_and_unknown_ids_are_refused() {
        let json = r#"[{"id":"okta","token_url":"https://okta.example/oauth2/v1/token"}]"#;
        let r = reader(&[("oauth_providers", json)]);
        assert_eq!(
            resolve("okta", None, &r).unwrap(),
            "https://okta.example/oauth2/v1/token"
        );
        assert_eq!(
            resolve("nope", None, &r),
            Err(TokenUrlError::UnknownProvider)
        );
        assert_eq!(
            resolve("anything", Some("https://evil.example"), &reader(&[])),
            Err(TokenUrlError::UnknownProvider)
        );
    }
}
