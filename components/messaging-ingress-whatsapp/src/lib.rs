mod bindings {
    wit_bindgen::generate!({
        path: "wit/messaging-ingress-whatsapp",
        world: "messaging-ingress-whatsapp",
        generate_all
    });
}

use bindings::exports::provider::common::ingress::Guest;
use bindings::greentic::secrets_store::secrets_store;
use serde_json::{Value, json};

const VERIFY_TOKEN_KEY: &str = "WHATSAPP_VERIFY_TOKEN";

struct Component;

impl Guest for Component {
    fn handle_webhook(headers_json: String, body_json: String) -> Result<String, String> {
        let _headers: Value = serde_json::from_str(&headers_json)
            .map_err(|_| "validation error: invalid headers".to_string())?;

        let parsed: Value = serde_json::from_str(&body_json)
            .map_err(|_| "validation error: invalid body".to_string())?;

        if let Some(token) = parsed
            .get("hub.verify_token")
            .or_else(|| parsed.get("verify_token"))
            .and_then(Value::as_str)
        {
            let expected = get_secret(VERIFY_TOKEN_KEY)?;
            if token != expected {
                return Err("validation error: verify token mismatch".into());
            }
        }

        let normalized = json!({ "ok": true, "event": parsed });
        serde_json::to_string(&normalized)
            .map_err(|_| "other error: serialization failed".to_string())
    }
}

bindings::exports::provider::common::ingress::__export_provider_common_ingress_0_0_2_cabi!(
    Component with_types_in bindings::exports::provider::common::ingress
);

fn get_secret(key: &str) -> Result<String, String> {
    match secrets_store::get(key) {
        Ok(Some(bytes)) => String::from_utf8(bytes).map_err(|_| "secret not valid utf-8".into()),
        Ok(None) => Err(format!("missing secret: {key}")),
        Err(e) => Err(format!("secret store error: {e:?}")),
    }
}
