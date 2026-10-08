//! Shared inbound-authenticity vectors (`fixtures/inbound-auth-v1/`). greentic-start
//! copies these files byte for byte, so both repos prove the same bytes.

use std::fs;
use std::path::PathBuf;

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use hmac::{Hmac, KeyInit, Mac};
use serde_json::Value;
use sha2::{Digest, Sha256};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/inbound-auth-v1")
}

fn load(name: &str) -> Value {
    serde_json::from_str(&fs::read_to_string(dir().join(name)).expect(name)).expect(name)
}

fn text<'a>(doc: &'a Value, key: &str) -> &'a str {
    doc[key].as_str().unwrap_or_else(|| panic!("missing {key}"))
}

fn body(doc: &Value) -> Vec<u8> {
    STANDARD
        .decode(text(doc, "body_base64"))
        .expect("body_base64")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn checksums_cover_every_vector_and_match() {
    let listed = fs::read_to_string(dir().join("CHECKSUMS.sha256")).expect("checksums");
    let mut names = Vec::new();
    for line in listed.lines() {
        let (digest, name) = line.split_once("  ").expect("sha256sum format");
        let actual = hex(&Sha256::digest(fs::read(dir().join(name)).expect(name)));
        assert_eq!(
            actual, digest,
            "{name} changed without updating CHECKSUMS.sha256"
        );
        names.push(name.to_string());
    }
    names.sort();
    assert_eq!(names, ["teams.json", "webex.json", "whatsapp.json"]);
}

#[test]
fn whatsapp_vector_is_hmac_sha256_over_the_raw_body() {
    let doc = load("whatsapp.json");
    let mut mac = Hmac::<Sha256>::new_from_slice(text(&doc, "secret").as_bytes()).expect("key");
    mac.update(&body(&doc));
    let expected = format!("sha256={}", hex(&mac.finalize().into_bytes()));
    assert_eq!(text(&doc, "header"), expected);
    assert_eq!(text(&doc, "header_name"), "x-hub-signature-256");
}

#[test]
fn webex_vector_is_hmac_sha1_over_the_raw_body() {
    let doc = load("webex.json");
    let key = ring::hmac::Key::new(
        ring::hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY,
        text(&doc, "secret").as_bytes(),
    );
    let tag = ring::hmac::sign(&key, &body(&doc));
    assert_eq!(text(&doc, "header"), hex(tag.as_ref()));
    assert_eq!(text(&doc, "header_name"), "x-spark-signature");
    assert_eq!(
        text(&doc, "secret").len(),
        20,
        "the pack generates 20 chars"
    );
}

// A verifier that parsed and re-serialised the body would not reproduce
// these bytes; the vectors keep JSON escapes so a test can prove it.
#[test]
fn hmac_vector_bodies_do_not_survive_reserialisation() {
    for name in ["whatsapp.json", "webex.json"] {
        let raw = body(&load(name));
        let parsed: Value = serde_json::from_slice(&raw).expect("body is json");
        assert_ne!(serde_json::to_vec(&parsed).expect("json"), raw, "{name}");
    }
}

#[test]
fn teams_vector_is_an_rs256_token_from_the_listed_key() {
    let doc = load("teams.json");
    let token = text(&doc, "header")
        .strip_prefix("Bearer ")
        .expect("Bearer token");
    let mut parts = token.split('.');
    let (header_b64, claims_b64, sig_b64) = (
        parts.next().expect("header"),
        parts.next().expect("claims"),
        parts.next().expect("signature"),
    );
    assert!(parts.next().is_none());

    let header: Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(header_b64).expect("b64")).expect("json");
    assert_eq!(header["alg"], "RS256");
    let jwk = &doc["jwks"]["keys"][0];
    assert_eq!(header["kid"], jwk["kid"]);
    assert_eq!(jwk["kty"], "RSA");

    let n = URL_SAFE_NO_PAD.decode(text(jwk, "n")).expect("n");
    let e = URL_SAFE_NO_PAD.decode(text(jwk, "e")).expect("e");
    let signing_input = format!("{header_b64}.{claims_b64}");
    ring::signature::RsaPublicKeyComponents { n: &n, e: &e }
        .verify(
            &ring::signature::RSA_PKCS1_2048_8192_SHA256,
            signing_input.as_bytes(),
            &URL_SAFE_NO_PAD.decode(sig_b64).expect("sig"),
        )
        .expect("signature verifies with the listed key");

    let claims: Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(claims_b64).expect("b64")).expect("json");
    assert_eq!(claims, doc["expected_claims"]);
    assert_eq!(claims["iss"], "https://api.botframework.com");
    assert_eq!(claims["aud"].as_str(), Some(text(&doc, "app_id")));
    let now = doc["now"].as_u64().expect("now");
    assert!(claims["nbf"].as_u64().expect("nbf") <= now);
    assert!(now < claims["exp"].as_u64().expect("exp"));

    let activity: Value = serde_json::from_slice(&body(&doc)).expect("activity");
    assert_eq!(claims["serviceurl"], activity["serviceUrl"]);
    let channel = activity["channelId"].as_str().expect("channelId");
    assert!(
        jwk["endorsements"]
            .as_array()
            .expect("endorsements")
            .iter()
            .any(|e| e.as_str() == Some(channel))
    );
}
