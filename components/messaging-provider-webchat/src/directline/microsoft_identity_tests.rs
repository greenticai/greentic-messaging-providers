use crate::directline::oidc_entra_test_support::{
    CLIENT, NONCE, OID, TID, claims, jwks, key, sign,
};

struct EntraFetcher {
    authority: &'static str,
    keys: String,
}

impl JwksFetcher for EntraFetcher {
    fn fetch(&self, url: &str) -> Result<String, String> {
        if url.ends_with("/v2.0/.well-known/openid-configuration") {
            assert!(url.contains(&format!("login.microsoftonline.com/{}/", self.authority)));
            let issuer = if self.authority.len() == 36 {
                format!("https://login.microsoftonline.com/{}/v2.0", self.authority)
            } else {
                "https://login.microsoftonline.com/{tenantid}/v2.0".to_string()
            };
            return Ok(json!({
                "issuer": issuer,
                "jwks_uri": format!(
                    "https://login.microsoftonline.com/{}/discovery/v2.0/keys",
                    self.authority
                ),
            })
            .to_string());
        }
        if url.ends_with("/discovery/v2.0/keys") {
            return Ok(self.keys.clone());
        }
        Err(format!("unexpected fetch {url}"))
    }
}

fn live_claims() -> Value {
    let now = Utc::now().timestamp();
    let mut c = claims();
    c["exp"] = json!(now + 3600);
    c["nbf"] = json!(now - 60);
    c
}

fn mint(
    config: Value,
    authority: &'static str,
    id_token: &str,
    nonce: Option<&str>,
    idp: bool,
) -> HttpOutV1 {
    let mut state = InMemoryStateStore::new();
    let mut secrets = TestSecretStore::new();
    secrets.insert(TOKEN_SECRET_KEY, b"test-signing-key");
    let mut request = token_request_with_config(config);
    request.headers.push(Header {
        name: "Authorization".into(),
        value: format!("Bearer {id_token}"),
    });
    if idp {
        request.headers.push(Header {
            name: IDP_HEADER.into(),
            value: "microsoft".into(),
        });
    }
    if let Some(nonce) = nonce {
        request.headers.push(Header {
            name: IDP_NONCE_HEADER.into(),
            value: nonce.into(),
        });
    }
    let fetcher = EntraFetcher {
        authority,
        keys: jwks(&[("k1", key(0))]),
    };
    handle_directline_request_with_jwks(&request, &mut state, &secrets, &fetcher)
}

fn answers(extra: Value) -> Value {
    let mut base = json!({
        "oauth_enabled": true,
        "oauth_enable_microsoft": true,
        "oauth_microsoft_client_id": CLIENT,
    });
    for (k, v) in extra.as_object().expect("object") {
        base[k] = v.clone();
    }
    base
}

fn minted_claims(response: &HttpOutV1) -> TokenClaims {
    assert_eq!(response.status, 200, "{:?}", decode_body(response));
    let body = decode_body(response).expect("json");
    verify_token(b"test-signing-key", body["token"].as_str().expect("token")).expect("verifies")
}

fn token() -> String {
    sign(
        key(0),
        &json!({"alg": "RS256", "typ": "JWT", "kid": "k1"}),
        &live_claims(),
    )
}

#[test]
fn a_verified_microsoft_token_mints_a_verified_ms_identity_with_a_caller_block() {
    let config = answers(json!({ "oauth_microsoft_authority": TID }));
    let response = mint(config, TID, &token(), Some(NONCE), true);
    let claims = minted_claims(&response);
    assert!(claims.verified);
    assert_eq!(claims.sub, format!("ms:{TID}:{OID}"));
    let block = crate::directline::caller::caller_block(&claims);
    assert_eq!(block["user_verified"], json!(true));
    assert_eq!(block["sub"], json!(format!("ms:{TID}:{OID}")));
    assert_eq!(
        block["iss"],
        json!(format!("https://login.microsoftonline.com/{TID}/v2.0"))
    );
    assert_eq!(block["tid"], json!(TID));
    assert_eq!(block["email"], json!("ada@contoso.com"));
    assert_eq!(block["name"], json!("Ada"));
}

#[test]
fn an_allow_listed_tenant_on_a_multi_tenant_authority_is_verified() {
    let config = answers(json!({
        "oauth_microsoft_authority": "organizations",
        "oauth_microsoft_allowed_tenants": TID,
    }));
    let response = mint(config, "organizations", &token(), Some(NONCE), true);
    assert!(minted_claims(&response).verified);
}

#[test]
fn a_tenant_outside_the_allow_list_is_refused_not_downgraded() {
    let config = answers(json!({
        "oauth_microsoft_authority": "organizations",
        "oauth_microsoft_allowed_tenants": "00000000-0000-0000-0000-000000000001",
    }));
    let response = mint(config, "organizations", &token(), Some(NONCE), true);
    assert_eq!(response.status, 401);
}

#[test]
fn common_with_an_empty_allow_list_falls_back_to_anonymous() {
    for config in [
        answers(json!({})),
        answers(json!({ "oauth_microsoft_authority": "organizations" })),
    ] {
        let response = mint(config, "common", &token(), Some(NONCE), true);
        let claims = minted_claims(&response);
        assert!(!claims.verified);
        assert!(!claims.sub.starts_with("ms:"));
    }
}

#[test]
fn a_wrong_nonce_or_a_missing_nonce_is_refused() {
    let config = answers(json!({ "oauth_microsoft_authority": TID }));
    assert_eq!(
        mint(config.clone(), TID, &token(), Some("other"), true).status,
        401
    );
    assert_eq!(mint(config, TID, &token(), None, true).status, 401);
}

#[test]
fn a_forged_token_is_refused() {
    let config = answers(json!({ "oauth_microsoft_authority": TID }));
    let forged = sign(
        key(1),
        &json!({"alg": "RS256", "typ": "JWT", "kid": "k1"}),
        &live_claims(),
    );
    assert_eq!(mint(config, TID, &forged, Some(NONCE), true).status, 401);
}

#[test]
fn without_the_idp_header_a_microsoft_token_never_becomes_an_identity() {
    let config = answers(json!({ "oauth_microsoft_authority": TID }));
    let response = mint(config, TID, &token(), Some(NONCE), false);
    assert_eq!(response.status, 401, "no greentic issuer is configured");
}
