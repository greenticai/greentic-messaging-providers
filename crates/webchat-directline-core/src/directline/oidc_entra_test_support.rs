#![cfg(test)]

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rsa::sha2::{Digest, Sha256};
use rsa::traits::PublicKeyParts;
use rsa::{Pkcs1v15Sign, RsaPrivateKey};
use serde_json::{Value, json};
use std::sync::OnceLock;

pub const TID: &str = "72f988bf-86f1-41af-91ab-2d7cd011db47";
pub const OID: &str = "11111111-2222-3333-4444-555555555555";
pub const CLIENT: &str = "ms-client";
pub const NONCE: &str = "nonce-123";
pub const NOW: i64 = 1_900_000_000;

pub fn key(slot: usize) -> &'static RsaPrivateKey {
    static KEYS: OnceLock<Vec<RsaPrivateKey>> = OnceLock::new();
    &KEYS.get_or_init(|| {
        (0..2)
            .map(|_| RsaPrivateKey::new(&mut rand_core::OsRng, 1024).expect("test key"))
            .collect()
    })[slot]
}

pub fn jwks(entries: &[(&str, &RsaPrivateKey)]) -> String {
    let keys: Vec<Value> = entries
        .iter()
        .map(|(kid, k)| {
            json!({
                "kty": "RSA", "use": "sig", "kid": kid,
                "n": URL_SAFE_NO_PAD.encode(k.n().to_bytes_be()),
                "e": URL_SAFE_NO_PAD.encode(k.e().to_bytes_be()),
            })
        })
        .collect();
    json!({ "keys": keys }).to_string()
}

pub fn sign(k: &RsaPrivateKey, header: &Value, claims: &Value) -> String {
    let h = URL_SAFE_NO_PAD.encode(header.to_string());
    let c = URL_SAFE_NO_PAD.encode(claims.to_string());
    let digest = Sha256::digest(format!("{h}.{c}").as_bytes());
    let sig = k
        .sign(Pkcs1v15Sign::new::<Sha256>(), &digest)
        .expect("sign");
    format!("{h}.{c}.{}", URL_SAFE_NO_PAD.encode(sig))
}

pub fn claims() -> Value {
    json!({
        "iss": format!("https://login.microsoftonline.com/{TID}/v2.0"),
        "aud": CLIENT, "tid": TID, "oid": OID, "nonce": NONCE,
        "exp": NOW + 3600, "nbf": NOW - 60,
        "preferred_username": "ada@contoso.com", "name": "Ada",
    })
}

pub fn header() -> Value {
    json!({"alg": "RS256", "typ": "JWT", "kid": "k1"})
}
