use provider_common::component_v0_6::{canonical_cbor_bytes, decode_cbor};
use provider_common::helpers::{
    cbor_json_invoke_bridge, json_bytes, schema_core_describe, schema_core_healthcheck,
    schema_core_validate_config,
};
use provider_common::qa_helpers::ApplyAnswersResult;
use serde_json::{Value, json};

mod bindings {
    wit_bindgen::generate!({
        path: "wit/messaging-provider-webchat",
        world: "component-v0-v6-v0",
        generate_all
    });
}
mod config;
mod describe;
mod directline;
mod ops;

use config::{ProviderConfigOut, apply_answers_merge};
use describe::{
    I18N_KEYS, I18N_PAIRS, build_describe_payload, build_qa_spec,
};
use ops::{
    encode_op, handle_ingest, handle_send, ingest_http, render_plan, send_payload,
};

const PROVIDER_ID: &str = "messaging-provider-webchat";
const PROVIDER_TYPE: &str = "messaging.webchat";
const WORLD_ID: &str = "component-v0-v6-v0";

struct Component;

impl bindings::exports::greentic::component::descriptor::Guest for Component {
    fn describe() -> Vec<u8> {
        canonical_cbor_bytes(&build_describe_payload())
    }
}

impl bindings::exports::greentic::component::runtime::Guest for Component {
    fn invoke(op: String, input_cbor: Vec<u8>) -> Vec<u8> {
        cbor_json_invoke_bridge(&op, &input_cbor, Some("send"), dispatch_json_invoke)
    }
}

impl bindings::exports::greentic::component::qa::Guest for Component {
    fn qa_spec(mode: bindings::exports::greentic::component::qa::Mode) -> Vec<u8> {
        canonical_cbor_bytes(&build_qa_spec(mode))
    }

    fn apply_answers(
        mode: bindings::exports::greentic::component::qa::Mode,
        answers_cbor: Vec<u8>,
    ) -> Vec<u8> {
        let answers: Value = match decode_cbor(&answers_cbor) {
            Ok(value) => value,
            Err(err) => {
                return canonical_cbor_bytes(
                    &ApplyAnswersResult::<ProviderConfigOut>::decode_error(format!(
                        "invalid answers cbor: {err}"
                    )),
                );
            }
        };

        if mode == bindings::exports::greentic::component::qa::Mode::Remove {
            return canonical_cbor_bytes(
                &ApplyAnswersResult::<ProviderConfigOut>::remove_default(),
            );
        }

        match apply_answers_merge(&mode, &answers) {
            Ok(merged) => canonical_cbor_bytes(&ApplyAnswersResult::success(merged)),
            Err(error) => canonical_cbor_bytes(
                &ApplyAnswersResult::<ProviderConfigOut>::validation_error(error),
            ),
        }
    }
}

impl bindings::exports::greentic::component::component_i18n::Guest for Component {
    fn i18n_keys() -> Vec<String> {
        provider_common::helpers::i18n_keys_from(I18N_KEYS)
    }

    fn i18n_bundle(locale: String) -> Vec<u8> {
        provider_common::helpers::i18n_bundle_from_pairs(locale, I18N_PAIRS)
    }
}

// Backward-compatible schema-core-api export for operator v0.4.x
impl bindings::exports::greentic::provider_schema_core::schema_core_api::Guest for Component {
    fn describe() -> Vec<u8> {
        schema_core_describe(&build_describe_payload())
    }

    fn validate_config(_config_json: Vec<u8>) -> Vec<u8> {
        schema_core_validate_config()
    }

    fn healthcheck() -> Vec<u8> {
        schema_core_healthcheck()
    }

    fn invoke(op: String, input_json: Vec<u8>) -> Vec<u8> {
        let op = if op == "run" { "send" } else { op.as_str() };
        dispatch_json_invoke(op, &input_json)
    }
}

bindings::export!(Component with_types_in bindings);

fn dispatch_json_invoke(op: &str, input_json: &[u8]) -> Vec<u8> {
    match op {
        "send" => handle_send(input_json),
        "ingest" => handle_ingest(input_json),
        "ingest_http" => ingest_http(input_json),
        "render_plan" => render_plan(input_json),
        "encode" => encode_op(input_json),
        "send_payload" => send_payload(input_json),
        other => json_bytes(&json!({"ok": false, "error": format!("unsupported op: {other}")})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    provider_common::standard_provider_tests! {
        describe_fn: build_describe_payload,
        qa_spec_fn: build_qa_spec,
        i18n_keys: I18N_KEYS,
        world_id: WORLD_ID,
        provider_id: PROVIDER_ID,
        schema_hash: "19cb56f3932284b00dc8938756534ff1deb7d58ebee08a7aeed3b8abf2e53a88",
        qa_default_keys: ["public_base_url"],
        mode_type: bindings::exports::greentic::component::qa::Mode,
        component_type: Component,
        qa_guest_path: bindings::exports::greentic::component::qa::Guest,
        validation_answers: {"public_base_url": "not-a-url"},
        validation_field: "public_base_url",
    }

    #[test]
    fn parse_config_requires_new_fields() {
        let cfg = br#"{"enabled":true,"public_base_url":"https://example.com","mode":"local_queue","route":"r"}"#;
        let parsed = config::parse_config_bytes(cfg).expect("valid config");
        assert!(parsed.enabled);
        assert_eq!(parsed.mode, "local_queue");
    }

    #[test]
    fn load_config_prefers_nested() {
        let input = json!({
            "config": {"enabled":true,"route":"inner","public_base_url":"https://example.com","mode":"local_queue"},
            "route": "outer"
        });
        let cfg = config::load_config(&input).unwrap();
        assert_eq!(cfg.route.as_deref(), Some("inner"));
        assert_eq!(cfg.public_base_url, "https://example.com");
    }

    #[test]
    fn parse_config_rejects_unknown() {
        let cfg = br#"{"enabled":true,"route":"r","public_base_url":"https://example.com","mode":"local_queue","extra":true}"#;
        let err = config::parse_config_bytes(cfg).unwrap_err();
        assert!(err.contains("unknown field"));
    }

    #[test]
    fn apply_answers_upgrade_preserves_unspecified_fields() {
        use bindings::exports::greentic::component::qa::Guest as QaGuest;
        use bindings::exports::greentic::component::qa::Mode;


        let answers = json!({
            "existing_config": {
                "enabled": true,
                "public_base_url": "https://example.com",
                "mode": "local_queue",
                "route": "/chat",
                "tenant_channel_id": "tenant-a",
                "base_url": "https://chat.example.com"
            },
            "route": "/messages"
        });
        let out =
            <Component as QaGuest>::apply_answers(Mode::Upgrade, canonical_cbor_bytes(&answers));
        let out_json: Value = decode_cbor(&out).expect("decode apply output");
        assert_eq!(out_json.get("ok"), Some(&Value::Bool(true)));
        let config = out_json.get("config").expect("config object");
        assert_eq!(
            config.get("tenant_channel_id"),
            Some(&Value::String("tenant-a".to_string()))
        );
        assert_eq!(
            config.get("route"),
            Some(&Value::String("/messages".to_string()))
        );
    }
}
