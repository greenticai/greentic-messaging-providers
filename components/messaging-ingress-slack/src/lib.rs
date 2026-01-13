mod bindings {
    wit_bindgen::generate!({
        path: "wit/messaging-ingress-slack",
        world: "messaging-ingress-slack",
        generate_all
    });
}

use bindings::exports::provider::common::ingress::Guest;
use bindings::greentic::secrets_store::secrets_store;
use hmac::{Hmac, Mac};
use serde_json::{Map, Value, json};
use sha2::Sha256;

const SIGNING_SECRET_KEY: &str = "SLACK_SIGNING_SECRET";

struct Component;

impl Guest for Component {
    fn handle_webhook(headers_json: String, body_json: String) -> Result<String, String> {
        let headers: Map<String, Value> = serde_json::from_str(&headers_json)
            .map_err(|_| "validation error: invalid headers".to_string())?;

        if let Some(secret_result) = get_optional_secret(SIGNING_SECRET_KEY) {
            let signing_secret = secret_result.map_err(|e| format!("transport error: {e}"))?;
            verify_signature(&headers, &body_json, &signing_secret)?;
        }

        let body_val: Value = serde_json::from_str(&body_json)
            .map_err(|_| "validation error: invalid body json".to_string())?;
        let normalized = json!({
            "ok": true,
            "event": body_val,
        });
        serde_json::to_string(&normalized)
            .map_err(|_| "other error: serialization failed".to_string())
    }
}

bindings::exports::provider::common::ingress::__export_provider_common_ingress_0_0_2_cabi!(
    Component with_types_in bindings::exports::provider::common::ingress
);

fn get_optional_secret(key: &str) -> Option<Result<String, String>> {
    match secrets_store::get(key) {
        Ok(Some(bytes)) => {
            Some(String::from_utf8(bytes).map_err(|_| "secret not valid utf-8".into()))
        }
        Ok(None) => None,
        Err(e) => Some(Err(format!("secret store error: {e:?}"))),
    }
}

fn verify_signature(headers: &Map<String, Value>, body: &str, secret: &str) -> Result<(), String> {
    let signature = header_value(headers, "x-slack-signature")
        .ok_or_else(|| "validation error: missing signature".to_string())?;
    let timestamp = header_value(headers, "x-slack-request-timestamp")
        .ok_or_else(|| "validation error: missing timestamp".to_string())?;

    let basestring = format!("v0:{timestamp}:{body}");
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .map_err(|_| "validation error: invalid secret".to_string())?;
    mac.update(basestring.as_bytes());
    let signature_bytes = mac.finalize().into_bytes();
    let computed = format!("v0={}", hex_encode(&signature_bytes));

    if computed == signature {
        Ok(())
    } else {
        Err("validation error: invalid signature".to_string())
    }
}

fn header_value(headers: &Map<String, Value>, key: &str) -> Option<String> {
    headers
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .and_then(|(_, v)| v.as_str())
                .map(|s| s.to_string())
        })
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{:02x}", b));
    }
    out
}
