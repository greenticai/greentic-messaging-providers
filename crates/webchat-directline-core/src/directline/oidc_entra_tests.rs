use super::*;
use crate::directline::oidc_entra_test_support::*;
use serde_json::json;

fn single_config() -> EntraConfig {
    EntraConfig {
        client_id: CLIENT.into(),
        authority: Authority::Single(TID.into()),
        allowed_tenants: vec![],
    }
}

fn multi_config(allowed: &[&str]) -> EntraConfig {
    EntraConfig {
        client_id: CLIENT.into(),
        authority: Authority::Multi("organizations".into()),
        allowed_tenants: allowed.iter().map(|s| s.to_string()).collect(),
    }
}

fn verify(token: &str, config: &EntraConfig, nonce: &str) -> Result<EntraIdentity, EntraError> {
    verify_at(token, config, nonce, NOW, Some(TID))
}

fn verify_at(
    token: &str,
    config: &EntraConfig,
    nonce: &str,
    now: i64,
    single: Option<&str>,
) -> Result<EntraIdentity, EntraError> {
    let doc = jwks(&[("k1", key(0))]);
    verify_id_token(
        token,
        &doc,
        &Expect {
            config,
            single_tenant: single,
            nonce,
            now,
        },
    )
}

#[test]
fn a_valid_token_yields_the_ms_subject_and_descriptive_claims() {
    let token = sign(key(0), &header(), &claims());
    let id = verify(&token, &single_config(), NONCE).expect("accepted");
    assert_eq!(id.sub, format!("ms:{TID}:{OID}"));
    assert_eq!(id.tid, TID);
    assert_eq!(id.email.as_deref(), Some("ada@contoso.com"));
    assert_eq!(id.name.as_deref(), Some("Ada"));
}

#[test]
fn the_second_jwks_key_is_found_by_kid() {
    let token = sign(key(1), &json!({"alg": "RS256", "kid": "k2"}), &claims());
    let doc = jwks(&[("k1", key(0)), ("k2", key(1))]);
    let config = single_config();
    let expect = Expect {
        config: &config,
        single_tenant: Some(TID),
        nonce: NONCE,
        now: NOW,
    };
    assert!(verify_id_token(&token, &doc, &expect).is_ok());
}

#[test]
fn a_wrong_audience_is_refused() {
    let mut c = claims();
    c["aud"] = json!("someone-else");
    let token = sign(key(0), &header(), &c);
    assert_eq!(
        verify(&token, &single_config(), NONCE),
        Err(EntraError::AudienceMismatch)
    );
}

#[test]
fn an_issuer_that_does_not_match_the_tokens_own_tenant_is_refused() {
    let mut c = claims();
    c["iss"] = json!("https://login.microsoftonline.com/00000000-0000-0000-0000-000000000000/v2.0");
    let token = sign(key(0), &header(), &c);
    assert_eq!(
        verify(&token, &single_config(), NONCE),
        Err(EntraError::IssuerMismatch)
    );
    c["iss"] = json!("https://evil.example/v2.0");
    let token = sign(key(0), &header(), &c);
    assert_eq!(
        verify(&token, &single_config(), NONCE),
        Err(EntraError::IssuerMismatch)
    );
}

#[test]
fn a_single_tenant_authority_refuses_another_tenant() {
    let other = "00000000-0000-0000-0000-000000000001";
    let mut c = claims();
    c["tid"] = json!(other);
    c["iss"] = json!(format!("https://login.microsoftonline.com/{other}/v2.0"));
    let token = sign(key(0), &header(), &c);
    assert_eq!(
        verify(&token, &single_config(), NONCE),
        Err(EntraError::TenantNotAllowed)
    );
}

#[test]
fn a_single_tenant_authority_without_a_resolved_tenant_refuses() {
    let token = sign(key(0), &header(), &claims());
    assert_eq!(
        verify_at(&token, &single_config(), NONCE, NOW, None),
        Err(EntraError::TenantNotAllowed)
    );
}

#[test]
fn an_expired_token_is_refused_but_small_skew_is_tolerated() {
    let token = sign(key(0), &header(), &claims());
    assert_eq!(
        verify_at(&token, &single_config(), NONCE, NOW + 3600 + 61, Some(TID)),
        Err(EntraError::Expired)
    );
    assert!(verify_at(&token, &single_config(), NONCE, NOW + 3600 + 30, Some(TID)).is_ok());
}

#[test]
fn a_token_not_yet_valid_is_refused() {
    let mut c = claims();
    c["nbf"] = json!(NOW + 600);
    let token = sign(key(0), &header(), &c);
    assert_eq!(
        verify(&token, &single_config(), NONCE),
        Err(EntraError::NotYetValid)
    );
}

#[test]
fn a_wrong_or_missing_nonce_is_refused() {
    let token = sign(key(0), &header(), &claims());
    assert_eq!(
        verify(&token, &single_config(), "other"),
        Err(EntraError::NonceMismatch)
    );
    assert_eq!(
        verify(&token, &single_config(), ""),
        Err(EntraError::NonceMismatch)
    );
    let mut c = claims();
    c.as_object_mut().expect("object").remove("nonce");
    let token = sign(key(0), &header(), &c);
    assert_eq!(
        verify(&token, &single_config(), NONCE),
        Err(EntraError::NonceMismatch)
    );
}

#[test]
fn a_multi_tenant_authority_admits_only_allow_listed_tenants() {
    let token = sign(key(0), &header(), &claims());
    assert!(verify(&token, &multi_config(&[TID]), NONCE).is_ok());
    assert_eq!(
        verify(
            &token,
            &multi_config(&["00000000-0000-0000-0000-000000000001"]),
            NONCE
        ),
        Err(EntraError::TenantNotAllowed)
    );
    assert_eq!(
        verify(&token, &multi_config(&[]), NONCE),
        Err(EntraError::TenantNotAllowed)
    );
}

#[test]
fn a_token_without_an_object_id_is_refused() {
    let mut c = claims();
    c.as_object_mut().expect("object").remove("oid");
    let token = sign(key(0), &header(), &c);
    assert_eq!(
        verify(&token, &single_config(), NONCE),
        Err(EntraError::MissingObjectId)
    );
}

#[test]
fn a_token_without_a_tenant_is_refused() {
    let mut c = claims();
    c.as_object_mut().expect("object").remove("tid");
    let token = sign(key(0), &header(), &c);
    assert_eq!(
        verify(&token, &single_config(), NONCE),
        Err(EntraError::MissingTenant)
    );
}

#[test]
fn a_signature_from_another_key_is_refused() {
    let token = sign(key(1), &header(), &claims());
    assert_eq!(
        verify(&token, &single_config(), NONCE),
        Err(EntraError::InvalidSignature)
    );
}

#[test]
fn a_tampered_payload_is_refused() {
    let token = sign(key(0), &header(), &claims());
    let mut parts: Vec<String> = token.split('.').map(str::to_string).collect();
    let mut c = claims();
    c["oid"] = json!("99999999-2222-3333-4444-555555555555");
    parts[1] = URL_SAFE_NO_PAD.encode(c.to_string());
    assert_eq!(
        verify(&parts.join("."), &single_config(), NONCE),
        Err(EntraError::InvalidSignature)
    );
}

#[test]
fn only_rs256_is_accepted() {
    for alg in ["none", "HS256", "ES256", "RS512"] {
        let token = sign(key(0), &json!({"alg": alg, "kid": "k1"}), &claims());
        assert_eq!(
            verify(&token, &single_config(), NONCE),
            Err(EntraError::UnsupportedAlg),
            "{alg}"
        );
    }
}

#[test]
fn an_unknown_kid_is_an_unknown_key() {
    let token = sign(key(0), &json!({"alg": "RS256", "kid": "nope"}), &claims());
    assert_eq!(
        verify(&token, &single_config(), NONCE),
        Err(EntraError::UnknownKey)
    );
}

#[test]
fn descriptive_claims_are_cleaned_and_bounded() {
    let mut c = claims();
    c["name"] = json!(format!("Ada\u{0}\n{}", "x".repeat(1000)));
    c["preferred_username"] = json!("   ");
    c["email"] = json!("ada@contoso.com");
    let token = sign(key(0), &header(), &c);
    let id = verify(&token, &single_config(), NONCE).expect("accepted");
    assert_eq!(id.email.as_deref(), Some("ada@contoso.com"));
    let name = id.name.expect("name");
    assert!(name.chars().count() <= MAX_DESCRIPTIVE_LEN && !name.contains('\n'));
}

fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: std::collections::HashMap<String, String> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    move |k| map.get(k).cloned()
}

const ENABLED: &[(&str, &str)] = &[
    ("oauth_enable_microsoft", "true"),
    ("oauth_microsoft_client_id", CLIENT),
];

fn with(extra: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static str)> {
    let mut v = ENABLED.to_vec();
    v.extend_from_slice(extra);
    v
}

#[test]
fn common_with_an_empty_allow_list_mints_nothing() {
    assert_eq!(resolve(lookup(ENABLED)), None);
    assert_eq!(
        resolve(lookup(&with(&[(
            "oauth_microsoft_authority",
            "organizations"
        )]))),
        None
    );
    assert_eq!(
        resolve(lookup(&with(&[("oauth_microsoft_allowed_tenants", " , ")]))),
        None
    );
}

#[test]
fn common_with_an_allow_list_is_multi_tenant() {
    let cfg = resolve(lookup(&with(&[(
        "oauth_microsoft_allowed_tenants",
        "72F988BF-86F1-41AF-91AB-2D7CD011DB47, 00000000-0000-0000-0000-000000000001",
    )])))
    .expect("configured");
    assert_eq!(cfg.authority, Authority::Multi("common".into()));
    assert_eq!(cfg.allowed_tenants[0], TID);
    assert_eq!(cfg.allowed_tenants.len(), 2);
}

#[test]
fn a_single_tenant_authority_needs_no_allow_list() {
    for authority in [TID, "contoso.onmicrosoft.com"] {
        let cfg = resolve(lookup(&with(&[("oauth_microsoft_authority", authority)])))
            .expect("configured");
        assert!(matches!(cfg.authority, Authority::Single(_)));
    }
}

#[test]
fn disabled_unconfigured_or_malformed_settings_mint_nothing() {
    assert_eq!(resolve(lookup(&[])), None);
    assert_eq!(
        resolve(lookup(&[("oauth_microsoft_client_id", CLIENT)])),
        None
    );
    assert_eq!(
        resolve(lookup(&with(&[
            ("oauth_enabled", "false"),
            ("oauth_microsoft_authority", TID)
        ]))),
        None
    );
    assert_eq!(
        resolve(lookup(&with(&[("oauth_microsoft_authority", "a/b")]))),
        None
    );
    assert_eq!(
        resolve(lookup(&with(&[
            ("oauth_microsoft_authority", TID),
            ("oauth_microsoft_allowed_tenants", "not-a-guid"),
        ]))),
        None,
        "a typo in the allow-list disables minting"
    );
}

#[test]
fn discovery_is_pinned_to_the_login_host() {
    let ok = json!({
        "jwks_uri": "https://login.microsoftonline.com/common/discovery/v2.0/keys",
        "issuer": "https://login.microsoftonline.com/{tenantid}/v2.0",
    })
    .to_string();
    let d = parse_discovery(&ok).expect("ok");
    assert_eq!(d.tenant, None);
    for bad in [
        "https://evil.example/keys",
        "http://login.microsoftonline.com/x",
        "https://login.microsoftonline.com.evil.example/keys",
        "https://login.microsoftonline.com/",
        "https://login.microsoftonline.com/x?y=1",
    ] {
        let doc = json!({ "jwks_uri": bad }).to_string();
        assert!(parse_discovery(&doc).is_err(), "{bad}");
    }
}

#[test]
fn a_domain_authority_takes_its_tenant_from_discovery() {
    let doc = json!({
        "jwks_uri": format!("https://login.microsoftonline.com/{TID}/discovery/v2.0/keys"),
        "issuer": format!("https://login.microsoftonline.com/{TID}/v2.0"),
    })
    .to_string();
    let d = parse_discovery(&doc).expect("ok");
    let domain = EntraConfig {
        authority: Authority::Single("contoso.onmicrosoft.com".into()),
        ..single_config()
    };
    assert_eq!(single_tenant(&domain, &d).as_deref(), Some(TID));
    assert_eq!(single_tenant(&single_config(), &d).as_deref(), Some(TID));
    let other = EntraConfig {
        authority: Authority::Single("00000000-0000-0000-0000-000000000001".into()),
        ..single_config()
    };
    assert_eq!(single_tenant(&other, &d), None);
}
