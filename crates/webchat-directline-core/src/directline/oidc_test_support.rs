#![cfg(test)]

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use p256::ecdsa::{Signature, SigningKey, signature::Signer};
use serde_json::Value;

pub fn generate_key() -> SigningKey {
    SigningKey::random(&mut rand_core::OsRng)
}

/// Derived from the public point, not attacker-controlled — distinct per key.
pub fn kid_for(signing_key: &SigningKey) -> String {
    let point = signing_key.verifying_key().to_encoded_point(false);
    URL_SAFE_NO_PAD.encode(&point.as_bytes()[1..9])
}

pub fn jwk_json(signing_key: &SigningKey, kid: &str) -> Value {
    let point = signing_key.verifying_key().to_encoded_point(false);
    serde_json::json!({
        "kty": "EC", "crv": "P-256", "alg": "ES256", "kid": kid,
        "x": URL_SAFE_NO_PAD.encode(&point.as_bytes()[1..33]),
        "y": URL_SAFE_NO_PAD.encode(&point.as_bytes()[33..65]),
    })
}

pub fn jwks_json(entries: &[Value]) -> String {
    serde_json::json!({ "keys": entries }).to_string()
}

/// Signs whatever header/claims JSON is handed in — lets tests forge
/// mismatched kid/key pairs and unusual `alg`/`typ` values.
pub fn sign_raw(signing_key: &SigningKey, header: &Value, claims: &Value) -> String {
    let header_enc = URL_SAFE_NO_PAD.encode(serde_json::to_vec(header).expect("header json"));
    let claims_enc = URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).expect("claims json"));
    let signing_input = format!("{header_enc}.{claims_enc}");
    let signature: Signature = signing_key.sign(signing_input.as_bytes());
    format!(
        "{signing_input}.{}",
        URL_SAFE_NO_PAD.encode(signature.to_bytes())
    )
}

pub fn claims_json(iss: &str, aud: &str, sub: &str, scope: &str, exp: i64) -> Value {
    serde_json::json!({
        "iss": iss, "aud": aud, "sub": sub, "scope": scope,
        "exp": exp, "iat": exp - 3600,
    })
}

/// Fresh P-256 keypair per call; each call mints a distinct `kid`.
pub fn signed_fixture(iss: &str, aud: &str, sub: &str, scope: &str, exp: i64) -> (String, String) {
    let signing_key = generate_key();
    let kid = kid_for(&signing_key);
    let header = serde_json::json!({"alg": "ES256", "typ": "JWT", "kid": kid});
    let claims = claims_json(iss, aud, sub, scope, exp);
    let token = sign_raw(&signing_key, &header, &claims);
    let jwks = jwks_json(&[jwk_json(&signing_key, &kid)]);
    (token, jwks)
}

/// Like [`signed_fixture`], with extra claims merged into the signed payload.
pub fn signed_fixture_with_claims(
    iss: &str,
    aud: &str,
    sub: &str,
    scope: &str,
    exp: i64,
    extra: Value,
) -> (String, String) {
    let signing_key = generate_key();
    let kid = kid_for(&signing_key);
    let header = serde_json::json!({"alg": "ES256", "typ": "JWT", "kid": kid});
    let mut claims = claims_json(iss, aud, sub, scope, exp);
    if let (Some(target), Some(source)) = (claims.as_object_mut(), extra.as_object()) {
        for (name, value) in source {
            target.insert(name.clone(), value.clone());
        }
    }
    let token = sign_raw(&signing_key, &header, &claims);
    let jwks = jwks_json(&[jwk_json(&signing_key, &kid)]);
    (token, jwks)
}
