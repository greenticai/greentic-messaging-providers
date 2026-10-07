//! Which OIDC issuer, audience and scope a Direct Line token mint verifies a
//! visitor's bearer against.
//!
//! Two pack-config shapes reach the mint, and both must configure the same
//! verification:
//!
//! * the provider's own `apply-answers` output, which writes `oidc_issuer` /
//!   `oidc_audience` (the WebChat GUI provider maps `oauth_greentic_issuer` →
//!   `oidc_issuer` and `oauth_greentic_client_id` → `oidc_audience` there);
//! * the RAW setup answers `greentic-setup` writes into
//!   `state/pack-configs/<pack>.json`. That tool never runs the provider's
//!   mapping, and `oidc_issuer` is not a setup question, so a bundle with
//!   Greentic SSO switched on carries only `oauth_enable_greentic`,
//!   `oauth_greentic_issuer` and `oauth_greentic_client_id`.
//!
//! Reading only `oidc_issuer` made every deployed bundle reject a signed-in
//! visitor's bearer with "oidc verification is not configured", so no
//! deployed WebChat visitor was ever `verified`.
//!
//! The rule mirrors the provider's mapping exactly, so the two shapes cannot
//! verify against different issuers:
//!
//! 1. an explicit `oidc_issuer` wins (it is what the mapping produces);
//! 2. otherwise `oauth_greentic_issuer` is used, but ONLY while Greentic SSO
//!    is switched on (`oauth_enable_greentic` true and `oauth_enabled` not
//!    false). A stale issuer answer under a switched-off section configures
//!    nothing.
//!
//! Every value comes from the tenant's own pack config; nothing here reads
//! the request. Issuer, audience and scope are still all checked by
//! `verify_access_token`, and the `https://` rule still applies to the issuer
//! whichever key it came from.

/// The verification a mint performs when a bearer is presented.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OidcVerification {
    pub issuer: String,
    pub audience: String,
    pub required_scope: String,
}

const DEFAULT_AUDIENCE: &str = "webchat-gui";
const DEFAULT_REQUIRED_SCOPE: &str = "greentic.webchat";

/// Resolves the verification from a pack-config lookup. `lookup` returns a
/// trimmed, non-empty value for a key (plain or host-injected `_b64`), or
/// `None`. Returns `None` when no issuer is configured.
pub fn resolve(lookup: impl Fn(&str) -> Option<String>) -> Option<OidcVerification> {
    let greentic_on = greentic_sso_enabled(&lookup);
    let issuer = lookup("oidc_issuer").or_else(|| {
        greentic_on
            .then(|| lookup("oauth_greentic_issuer"))
            .flatten()
    })?;
    let audience = lookup("oidc_audience")
        .or_else(|| {
            greentic_on
                .then(|| lookup("oauth_greentic_client_id"))
                .flatten()
        })
        .unwrap_or_else(|| DEFAULT_AUDIENCE.to_string());
    let required_scope =
        lookup("oidc_required_scope").unwrap_or_else(|| DEFAULT_REQUIRED_SCOPE.to_string());
    Some(OidcVerification {
        issuer,
        audience,
        required_scope,
    })
}

fn greentic_sso_enabled(lookup: &impl Fn(&str) -> Option<String>) -> bool {
    let master_off = lookup("oauth_enabled").is_some_and(|value| !is_true(&value));
    !master_off && lookup("oauth_enable_greentic").is_some_and(|value| is_true(&value))
}

fn is_true(value: &str) -> bool {
    value.eq_ignore_ascii_case("true")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup_in(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key: &str| map.get(key).cloned()
    }

    const SSO_ON: &[(&str, &str)] = &[
        ("oauth_enabled", "true"),
        ("oauth_enable_greentic", "true"),
        ("oauth_greentic_issuer", "https://acme.greentic-id.com"),
        ("oauth_greentic_client_id", "acme-webchat"),
    ];

    #[test]
    fn greentic_answers_resolve_issuer_and_audience() {
        let resolved = resolve(lookup_in(SSO_ON)).expect("configured");
        assert_eq!(resolved.issuer, "https://acme.greentic-id.com");
        assert_eq!(resolved.audience, "acme-webchat");
        assert_eq!(resolved.required_scope, "greentic.webchat");
    }

    #[test]
    fn nothing_is_configured_without_an_issuer() {
        assert_eq!(resolve(lookup_in(&[])), None);
        assert_eq!(
            resolve(lookup_in(&[("oauth_enable_greentic", "true")])),
            None
        );
    }

    #[test]
    fn a_switched_off_section_configures_nothing() {
        let mut off = SSO_ON.to_vec();
        off[1] = ("oauth_enable_greentic", "false");
        assert_eq!(resolve(lookup_in(&off)), None);

        let mut master_off = SSO_ON.to_vec();
        master_off[0] = ("oauth_enabled", "false");
        assert_eq!(resolve(lookup_in(&master_off)), None);

        let toggle_absent: Vec<_> = SSO_ON
            .iter()
            .copied()
            .filter(|(k, _)| *k != "oauth_enable_greentic")
            .collect();
        assert_eq!(resolve(lookup_in(&toggle_absent)), None);
    }

    #[test]
    fn an_absent_master_switch_does_not_block_greentic_sso() {
        let without_master: Vec<_> = SSO_ON
            .iter()
            .copied()
            .filter(|(k, _)| *k != "oauth_enabled")
            .collect();
        assert!(resolve(lookup_in(&without_master)).is_some());
    }

    #[test]
    fn explicit_oidc_keys_win() {
        let mut explicit = SSO_ON.to_vec();
        explicit.push(("oidc_issuer", "https://other.example"));
        explicit.push(("oidc_audience", "other-aud"));
        explicit.push(("oidc_required_scope", "custom.scope"));
        let resolved = resolve(lookup_in(&explicit)).expect("configured");
        assert_eq!(resolved.issuer, "https://other.example");
        assert_eq!(resolved.audience, "other-aud");
        assert_eq!(resolved.required_scope, "custom.scope");
    }

    #[test]
    fn the_greentic_client_id_is_ignored_while_sso_is_off() {
        let resolved = resolve(lookup_in(&[
            ("oidc_issuer", "https://other.example"),
            ("oauth_greentic_client_id", "acme-webchat"),
        ]))
        .expect("configured");
        assert_eq!(resolved.audience, "webchat-gui");
    }
}
