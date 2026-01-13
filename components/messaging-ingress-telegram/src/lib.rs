mod bindings {
    wit_bindgen::generate!({
        path: "wit/messaging-ingress-telegram",
        world: "messaging-ingress-telegram",
        generate_all
    });
}

use bindings::exports::provider::common::ingress::Guest;
use serde_json::{Value, json};

struct Component;

impl Guest for Component {
    fn handle_webhook(_headers_json: String, body_json: String) -> Result<String, String> {
        let parsed: Value = serde_json::from_str(&body_json)
            .map_err(|_| "validation error: invalid body".to_string())?;
        let normalized = json!({ "ok": true, "event": parsed });
        serde_json::to_string(&normalized)
            .map_err(|_| "other error: serialization failed".to_string())
    }
}

bindings::exports::provider::common::ingress::__export_provider_common_ingress_0_0_2_cabi!(
    Component with_types_in bindings::exports::provider::common::ingress
);
