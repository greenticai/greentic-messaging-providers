use super::verify_webex_signature;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use greentic_types::messaging::universal_dto::Header;
use serde_json::Value;

const VECTOR: &str =
    include_str!("../../../../crates/provider-tests/tests/fixtures/inbound-auth-v1/webex.json");

fn vector() -> (String, Vec<u8>, String) {
    let doc: Value = serde_json::from_str(VECTOR).expect("vector json");
    let field = |key: &str| doc[key].as_str().expect(key).to_string();
    let body = STANDARD.decode(field("body_base64")).expect("body");
    (field("secret"), body, field("header"))
}

fn header(name: &str, value: &str) -> Vec<Header> {
    vec![Header {
        name: name.to_string(),
        value: value.to_string(),
    }]
}

#[test]
fn the_shared_vector_verifies_with_the_providers_own_check() {
    let (secret, body, signature) = vector();
    assert!(verify_webex_signature(
        &header("X-Spark-Signature", &signature),
        &body,
        &secret
    ));
    assert!(verify_webex_signature(
        &header("x-webex-signature", &signature),
        &body,
        &secret
    ));
}

#[test]
fn one_changed_body_byte_fails_the_shared_vector() {
    let (secret, mut body, signature) = vector();
    body[0] ^= 1;
    assert!(!verify_webex_signature(
        &header("x-spark-signature", &signature),
        &body,
        &secret
    ));
}
