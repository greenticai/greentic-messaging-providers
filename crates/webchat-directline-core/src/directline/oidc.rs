use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use p256::EncodedPoint;
use p256::ecdsa::{Signature, VerifyingKey, signature::Verifier};
use serde::Deserialize;

#[derive(Debug)]
pub struct VerifiedIdentity {
    pub sub: String,
    /// The issuer's raw `teams` claim from the verified access token. It is
    /// unvalidated here (any JSON shape); `caller::verified_teams` is the one
    /// place that decides which slugs survive.
    pub teams: Option<serde_json::Value>,
}

#[derive(Debug)]
pub enum OidcError {
    InvalidFormat,
    UnsupportedAlg,
    InvalidTokenType,
    UnknownKey,
    InvalidSignature,
    Expired,
    NotYetValid,
    IssuerMismatch,
    AudienceMismatch,
    MissingScope,
    NoRequiredScope,
}

#[derive(Deserialize)]
struct JwtHeader {
    alg: String,
    #[serde(default)]
    typ: Option<String>,
    #[serde(default)]
    kid: Option<String>,
}

#[derive(Deserialize)]
struct AccessClaims {
    iss: String,
    sub: String,
    aud: String,
    #[serde(default)]
    scope: String,
    exp: i64,
    #[serde(default)]
    nbf: Option<i64>,
    #[serde(default)]
    teams: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct Jwk {
    kty: String,
    #[serde(default)]
    crv: Option<String>,
    #[serde(default)]
    kid: Option<String>,
    #[serde(default)]
    x: Option<String>,
    #[serde(default)]
    y: Option<String>,
}

#[derive(Deserialize)]
struct Jwks {
    keys: Vec<Jwk>,
}

/// Bounds candidate-key iteration if the JWKS source ever becomes tenant-configurable.
const MAX_JWKS_KEYS: usize = 16;

fn decode_part<T: for<'de> Deserialize<'de>>(part: &str) -> Result<T, OidcError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(part)
        .map_err(|_| OidcError::InvalidFormat)?;
    serde_json::from_slice(&bytes).map_err(|_| OidcError::InvalidFormat)
}

fn verifying_key(jwk: &Jwk) -> Result<VerifyingKey, OidcError> {
    if jwk.kty != "EC" || jwk.crv.as_deref() != Some("P-256") {
        return Err(OidcError::UnsupportedAlg);
    }
    let x = jwk.x.as_deref().ok_or(OidcError::UnknownKey)?;
    let y = jwk.y.as_deref().ok_or(OidcError::UnknownKey)?;
    let x_bytes = URL_SAFE_NO_PAD
        .decode(x)
        .map_err(|_| OidcError::UnknownKey)?;
    let y_bytes = URL_SAFE_NO_PAD
        .decode(y)
        .map_err(|_| OidcError::UnknownKey)?;
    if x_bytes.len() != 32 || y_bytes.len() != 32 {
        return Err(OidcError::UnknownKey);
    }
    let point = EncodedPoint::from_affine_coordinates(
        x_bytes.as_slice().into(),
        y_bytes.as_slice().into(),
        false,
    );
    VerifyingKey::from_encoded_point(&point).map_err(|_| OidcError::UnknownKey)
}

pub fn verify_access_token(
    token: &str,
    jwks_json: &str,
    expected_iss: &str,
    expected_aud: &str,
    required_scope: &str,
    now: i64,
) -> Result<VerifiedIdentity, OidcError> {
    if required_scope.is_empty() {
        return Err(OidcError::NoRequiredScope);
    }

    let mut parts = token.split('.');
    let header_enc = parts.next().ok_or(OidcError::InvalidFormat)?;
    let claims_enc = parts.next().ok_or(OidcError::InvalidFormat)?;
    let sig_enc = parts.next().ok_or(OidcError::InvalidFormat)?;
    if parts.next().is_some() {
        return Err(OidcError::InvalidFormat);
    }

    let header: JwtHeader = decode_part(header_enc)?;
    if header.alg != "ES256" {
        return Err(OidcError::UnsupportedAlg);
    }
    if header.typ.as_deref() != Some("JWT") {
        return Err(OidcError::InvalidTokenType);
    }

    let jwks: Jwks = serde_json::from_str(jwks_json).map_err(|_| OidcError::UnknownKey)?;
    if jwks.keys.len() > MAX_JWKS_KEYS {
        return Err(OidcError::UnknownKey);
    }
    let candidates: Vec<&Jwk> = jwks
        .keys
        .iter()
        .filter(|k| match (&header.kid, &k.kid) {
            (Some(want), Some(have)) => want == have,
            (Some(_), None) => false,
            (None, _) => true,
        })
        .collect();
    if candidates.is_empty() {
        return Err(OidcError::UnknownKey);
    }

    let sig_bytes = URL_SAFE_NO_PAD
        .decode(sig_enc)
        .map_err(|_| OidcError::InvalidFormat)?;
    let signature = Signature::from_slice(&sig_bytes).map_err(|_| OidcError::InvalidSignature)?;
    let signing_input = format!("{header_enc}.{claims_enc}");

    // Every key in the JWKS is a key the issuer asserts as its own, so
    // accepting a signature from any candidate is the correct trust model —
    // this only widens which of the issuer's own keys can verify, never
    // what counts as a valid signature.
    let mut saw_valid_key = false;
    let mut verified = false;
    for candidate in &candidates {
        match verifying_key(candidate) {
            Ok(key) => {
                saw_valid_key = true;
                if key.verify(signing_input.as_bytes(), &signature).is_ok() {
                    verified = true;
                    break;
                }
            }
            Err(_) => continue,
        }
    }
    if !verified {
        return Err(if saw_valid_key {
            OidcError::InvalidSignature
        } else {
            OidcError::UnknownKey
        });
    }

    let claims: AccessClaims = decode_part(claims_enc)?;
    if claims.iss != expected_iss {
        return Err(OidcError::IssuerMismatch);
    }
    if claims.aud != expected_aud {
        return Err(OidcError::AudienceMismatch);
    }
    if now >= claims.exp {
        return Err(OidcError::Expired);
    }
    if let Some(nbf) = claims.nbf
        && now < nbf
    {
        return Err(OidcError::NotYetValid);
    }
    if !claims.scope.split_whitespace().any(|s| s == required_scope) {
        return Err(OidcError::MissingScope);
    }

    Ok(VerifiedIdentity {
        sub: claims.sub,
        teams: claims.teams,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ISS: &str = "https://acme.greentic-id.com";
    const AUD: &str = "webchat-gui";
    const SCOPE: &str = "greentic.webchat";

    fn fixture() -> (String, String) {
        crate::directline::oidc_test_support::signed_fixture(
            ISS,
            AUD,
            "acme:users:1",
            "openid profile email greentic.webchat",
            2_000_000_000,
        )
    }

    #[test]
    fn accepts_a_valid_token() {
        let (token, jwks) = fixture();
        let id = verify_access_token(&token, &jwks, ISS, AUD, SCOPE, 1_900_000_000)
            .expect("token accepted");
        assert_eq!(id.sub, "acme:users:1");
    }

    #[test]
    fn rejects_an_expired_token() {
        let (token, jwks) = fixture();
        let err = verify_access_token(&token, &jwks, ISS, AUD, SCOPE, 2_000_000_001).unwrap_err();
        assert!(matches!(err, OidcError::Expired));
    }

    #[test]
    fn rejects_a_foreign_issuer() {
        let (token, jwks) = fixture();
        let err = verify_access_token(
            &token,
            &jwks,
            "https://evil.example",
            AUD,
            SCOPE,
            1_900_000_000,
        )
        .unwrap_err();
        assert!(matches!(err, OidcError::IssuerMismatch));
    }

    #[test]
    fn rejects_a_foreign_audience() {
        let (token, jwks) = fixture();
        let err = verify_access_token(&token, &jwks, ISS, "someone-else", SCOPE, 1_900_000_000)
            .unwrap_err();
        assert!(matches!(err, OidcError::AudienceMismatch));
    }

    #[test]
    fn rejects_a_token_without_the_required_scope() {
        let (token, jwks) = crate::directline::oidc_test_support::signed_fixture(
            ISS,
            AUD,
            "acme:users:1",
            "openid profile email",
            2_000_000_000,
        );
        let err = verify_access_token(&token, &jwks, ISS, AUD, SCOPE, 1_900_000_000).unwrap_err();
        assert!(matches!(err, OidcError::MissingScope));
    }

    #[test]
    fn rejects_a_tampered_signature() {
        let (token, jwks) = fixture();
        let mut parts: Vec<&str> = token.split('.').collect();
        let tampered_sig = "AAAA".to_string() + &parts[2][4..];
        parts[2] = &tampered_sig;
        let bad = parts.join(".");
        let err = verify_access_token(&bad, &jwks, ISS, AUD, SCOPE, 1_900_000_000).unwrap_err();
        assert!(matches!(err, OidcError::InvalidSignature));
    }

    #[test]
    fn rejects_a_key_the_jwks_does_not_carry() {
        let (token, _) = fixture();
        let (_, other_jwks) = crate::directline::oidc_test_support::signed_fixture(
            ISS,
            AUD,
            "acme:users:2",
            "greentic.webchat",
            2_000_000_000,
        );
        let err =
            verify_access_token(&token, &other_jwks, ISS, AUD, SCOPE, 1_900_000_000).unwrap_err();
        assert!(matches!(err, OidcError::UnknownKey));
    }

    #[test]
    fn rejects_an_empty_required_scope_argument() {
        let (token, jwks) = fixture();
        let err = verify_access_token(&token, &jwks, ISS, AUD, "", 1_900_000_000).unwrap_err();
        assert!(matches!(err, OidcError::NoRequiredScope));
    }

    #[test]
    fn rejects_a_token_with_a_non_jwt_typ() {
        use crate::directline::oidc_test_support::{
            claims_json, generate_key, jwk_json, jwks_json, kid_for, sign_raw,
        };
        let key = generate_key();
        let kid = kid_for(&key);
        let header = serde_json::json!({"alg": "ES256", "typ": "at+jwt", "kid": kid});
        let claims = claims_json(ISS, AUD, "acme:users:1", SCOPE, 2_000_000_000);
        let token = sign_raw(&key, &header, &claims);
        let jwks = jwks_json(&[jwk_json(&key, &kid)]);
        let err = verify_access_token(&token, &jwks, ISS, AUD, SCOPE, 1_900_000_000).unwrap_err();
        assert!(matches!(err, OidcError::InvalidTokenType));
    }

    #[test]
    fn rejects_a_signature_from_a_different_key_under_a_matching_kid() {
        use crate::directline::oidc_test_support::{
            claims_json, generate_key, jwk_json, jwks_json, kid_for, sign_raw,
        };
        let real_key = generate_key();
        let attacker_key = generate_key();
        let kid = kid_for(&real_key);
        let header = serde_json::json!({"alg": "ES256", "typ": "JWT", "kid": kid});
        let claims = claims_json(ISS, AUD, "acme:users:1", SCOPE, 2_000_000_000);
        // Signed by the attacker's key, but the `kid` claims to be the real one.
        let token = sign_raw(&attacker_key, &header, &claims);
        let jwks = jwks_json(&[jwk_json(&real_key, &kid)]);
        let err = verify_access_token(&token, &jwks, ISS, AUD, SCOPE, 1_900_000_000).unwrap_err();
        assert!(matches!(err, OidcError::InvalidSignature));
    }

    #[test]
    fn rejects_alg_none() {
        use crate::directline::oidc_test_support::{
            claims_json, generate_key, jwk_json, jwks_json, kid_for, sign_raw,
        };
        let key = generate_key();
        let kid = kid_for(&key);
        let header = serde_json::json!({"alg": "none", "typ": "JWT", "kid": kid});
        let claims = claims_json(ISS, AUD, "acme:users:1", SCOPE, 2_000_000_000);
        let token = sign_raw(&key, &header, &claims);
        let jwks = jwks_json(&[jwk_json(&key, &kid)]);
        let err = verify_access_token(&token, &jwks, ISS, AUD, SCOPE, 1_900_000_000).unwrap_err();
        assert!(matches!(err, OidcError::UnsupportedAlg));
    }

    #[test]
    fn rejects_alg_hs256() {
        use crate::directline::oidc_test_support::{
            claims_json, generate_key, jwk_json, jwks_json, kid_for, sign_raw,
        };
        let key = generate_key();
        let kid = kid_for(&key);
        let header = serde_json::json!({"alg": "HS256", "typ": "JWT", "kid": kid});
        let claims = claims_json(ISS, AUD, "acme:users:1", SCOPE, 2_000_000_000);
        let token = sign_raw(&key, &header, &claims);
        let jwks = jwks_json(&[jwk_json(&key, &kid)]);
        let err = verify_access_token(&token, &jwks, ISS, AUD, SCOPE, 1_900_000_000).unwrap_err();
        assert!(matches!(err, OidcError::UnsupportedAlg));
    }

    #[test]
    fn accepts_a_token_whose_key_is_second_in_the_jwks_with_a_matching_kid() {
        use crate::directline::oidc_test_support::{
            claims_json, generate_key, jwk_json, jwks_json, kid_for, sign_raw,
        };
        let key1 = generate_key();
        let kid1 = kid_for(&key1);
        let key2 = generate_key();
        let kid2 = kid_for(&key2);
        let header = serde_json::json!({"alg": "ES256", "typ": "JWT", "kid": kid2});
        let claims = claims_json(ISS, AUD, "acme:users:2", SCOPE, 2_000_000_000);
        let token = sign_raw(&key2, &header, &claims);
        let jwks = jwks_json(&[jwk_json(&key1, &kid1), jwk_json(&key2, &kid2)]);
        let id = verify_access_token(&token, &jwks, ISS, AUD, SCOPE, 1_900_000_000)
            .expect("second key in jwks accepted");
        assert_eq!(id.sub, "acme:users:2");
    }

    #[test]
    fn accepts_a_kid_less_token_signed_by_the_second_jwks_key() {
        use crate::directline::oidc_test_support::{
            claims_json, generate_key, jwk_json, jwks_json, kid_for, sign_raw,
        };
        let key1 = generate_key();
        let kid1 = kid_for(&key1);
        let key2 = generate_key();
        let kid2 = kid_for(&key2);
        let header = serde_json::json!({"alg": "ES256", "typ": "JWT"});
        let claims = claims_json(ISS, AUD, "acme:users:2", SCOPE, 2_000_000_000);
        let token = sign_raw(&key2, &header, &claims);
        let jwks = jwks_json(&[jwk_json(&key1, &kid1), jwk_json(&key2, &kid2)]);
        let id = verify_access_token(&token, &jwks, ISS, AUD, SCOPE, 1_900_000_000)
            .expect("no-kid token verified against every jwks candidate");
        assert_eq!(id.sub, "acme:users:2");
    }

    #[test]
    fn rejects_a_jwks_with_too_many_keys() {
        use crate::directline::oidc_test_support::{
            claims_json, generate_key, jwk_json, jwks_json, kid_for, sign_raw,
        };
        let key = generate_key();
        let kid = kid_for(&key);
        let header = serde_json::json!({"alg": "ES256", "typ": "JWT", "kid": kid});
        let claims = claims_json(ISS, AUD, "acme:users:1", SCOPE, 2_000_000_000);
        let token = sign_raw(&key, &header, &claims);
        let mut entries: Vec<serde_json::Value> = (0..16)
            .map(|_| jwk_json(&generate_key(), "padding"))
            .collect();
        entries.push(jwk_json(&key, &kid));
        let jwks = jwks_json(&entries);
        let err = verify_access_token(&token, &jwks, ISS, AUD, SCOPE, 1_900_000_000).unwrap_err();
        assert!(matches!(err, OidcError::UnknownKey));
    }
}
