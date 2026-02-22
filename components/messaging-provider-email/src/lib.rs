use provider_common::component_v0_6::{canonical_cbor_bytes, decode_cbor};
use provider_common::helpers::{
    cbor_json_invoke_bridge, existing_config_from_answers, json_bytes,
    optional_string_from, schema_core_describe, schema_core_healthcheck,
    schema_core_validate_config, string_or_default,
};
use provider_common::qa_helpers::ApplyAnswersResult;
use serde_json::{Value, json};

mod bindings {
    wit_bindgen::generate!({
        path: "wit/messaging-provider-email",
        world: "component-v0-v6-v0",
        generate_all
    });
}

mod auth;
mod config;
mod describe;
mod graph;
mod ingress;
mod ops;

use config::{ProviderConfigOut, default_config_out, validate_config_out};
use describe::{I18N_KEYS, build_describe_payload, build_qa_spec};
use graph::{subscription_delete, subscription_ensure, subscription_renew};
use ingress::ingest_http;
use ops::{encode_op, handle_reply, handle_send, render_plan, send_payload};

const PROVIDER_ID: &str = "messaging-provider-email";
const PROVIDER_TYPE: &str = "messaging.email.smtp";
const WORLD_ID: &str = "component-v0-v6-v0";
const DEFAULT_GRAPH_BASE: &str = "https://graph.microsoft.com/v1.0";
const GRAPH_MAX_EXPIRATION_MINUTES: u32 = 4230;

struct Component;

impl bindings::exports::greentic::component::descriptor::Guest for Component {
    fn describe() -> Vec<u8> {
        canonical_cbor_bytes(&build_describe_payload())
    }
}

impl bindings::exports::greentic::component::runtime::Guest for Component {
    fn invoke(op: String, input_cbor: Vec<u8>) -> Vec<u8> {
        cbor_json_invoke_bridge(&op, &input_cbor, Some("send"), |op, input| {
            dispatch_json_invoke(op, input)
        })
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

        let mut merged = existing_config_from_answers(&answers).unwrap_or_else(default_config_out);
        let answer_obj = answers.as_object();
        let has = |key: &str| answer_obj.is_some_and(|obj| obj.contains_key(key));

        if mode == bindings::exports::greentic::component::qa::Mode::Setup
            || mode == bindings::exports::greentic::component::qa::Mode::Default
        {
            merged.enabled = answers
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(merged.enabled);
            merged.public_base_url =
                string_or_default(&answers, "public_base_url", &merged.public_base_url);
            merged.host = string_or_default(&answers, "host", &merged.host);
            merged.port = answers
                .get("port")
                .and_then(Value::as_u64)
                .and_then(|value| u16::try_from(value).ok())
                .unwrap_or(merged.port);
            merged.username = string_or_default(&answers, "username", &merged.username);
            merged.from_address = string_or_default(&answers, "from_address", &merged.from_address);
            merged.tls_mode = string_or_default(&answers, "tls_mode", &merged.tls_mode);
            merged.default_to_address = optional_string_from(&answers, "default_to_address")
                .or(merged.default_to_address.clone());
            merged.password =
                optional_string_from(&answers, "password").or(merged.password.clone());
        }

        if mode == bindings::exports::greentic::component::qa::Mode::Upgrade {
            if has("enabled") {
                merged.enabled = answers
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(merged.enabled);
            }
            if has("public_base_url") {
                merged.public_base_url =
                    string_or_default(&answers, "public_base_url", &merged.public_base_url);
            }
            if has("host") {
                merged.host = string_or_default(&answers, "host", &merged.host);
            }
            if has("port") {
                merged.port = answers
                    .get("port")
                    .and_then(Value::as_u64)
                    .and_then(|value| u16::try_from(value).ok())
                    .unwrap_or(merged.port);
            }
            if has("username") {
                merged.username = string_or_default(&answers, "username", &merged.username);
            }
            if has("from_address") {
                merged.from_address =
                    string_or_default(&answers, "from_address", &merged.from_address);
            }
            if has("tls_mode") {
                merged.tls_mode = string_or_default(&answers, "tls_mode", &merged.tls_mode);
            }
            if has("default_to_address") {
                merged.default_to_address = optional_string_from(&answers, "default_to_address");
            }
            if has("password") {
                merged.password = optional_string_from(&answers, "password");
            }
        }

        if let Err(error) = validate_config_out(&merged) {
            return canonical_cbor_bytes(
                &ApplyAnswersResult::<ProviderConfigOut>::validation_error(error),
            );
        }

        canonical_cbor_bytes(&ApplyAnswersResult::success(merged))
    }
}

impl bindings::exports::greentic::component::component_i18n::Guest for Component {
    fn i18n_keys() -> Vec<String> {
        provider_common::helpers::i18n_keys_from(I18N_KEYS)
    }

    fn i18n_bundle(locale: String) -> Vec<u8> {
        describe::i18n_bundle(locale)
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
        let op = if op == "run" { "send".to_string() } else { op };
        dispatch_json_invoke(&op, &input_json)
    }
}

bindings::export!(Component with_types_in bindings);

fn dispatch_json_invoke(op: &str, input_json: &[u8]) -> Vec<u8> {
    match op {
        "send" => handle_send(input_json),
        "reply" => handle_reply(input_json),
        "ingest_http" => ingest_http(input_json),
        "render_plan" => render_plan(input_json),
        "encode" => encode_op(input_json),
        "send_payload" => send_payload(input_json),
        "subscription_ensure" => subscription_ensure(input_json),
        "subscription_renew" => subscription_renew(input_json),
        "subscription_delete" => subscription_delete(input_json),
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
        schema_hash: "a022076adb33dab084ad655fb83b4857a9d4aa7fd81b1d4d694a509789a63890",
        qa_default_keys: ["public_base_url", "host", "username", "from_address"],
        mode_type: bindings::exports::greentic::component::qa::Mode,
        component_type: Component,
        qa_guest_path: bindings::exports::greentic::component::qa::Guest,
        validation_answers: {"public_base_url": "not-a-url", "host": "smtp.example.com", "username": "user-a", "from_address": "from@example.com"},
        validation_field: "public_base_url",
    }
    #[test]
    fn parse_config_requires_new_fields() {
        let cfg = br#"{"enabled":true,"public_base_url":"https://example.com","host":"smtp.example.com","port":587,"username":"u","from_address":"from@example.com","tls_mode":"starttls"}"#;
        let parsed = config::parse_config_bytes(cfg).expect("valid config");
        assert!(parsed.enabled);
    }

    #[test]
    fn parse_config_rejects_unknown() {
        let cfg = br#"{"enabled":true,"public_base_url":"https://example.com","host":"smtp","port":587,"username":"u","from_address":"f","tls_mode":"starttls","unknown":true}"#;
        let err = config::parse_config_bytes(cfg).unwrap_err();
        assert!(err.contains("unknown field"));
    }

    #[test]
    fn load_config_prefers_nested() {
        let input = json!({
            "config": {
                "enabled": true,
                "public_base_url": "https://example.com",
                "host":"a",
                "port":25,
                "username":"u",
                "from_address":"f",
                "tls_mode":"starttls"
            },
            "host": "b"
        });
        let cfg = config::load_config(&input).unwrap();
        assert_eq!(cfg.host, "a");
        assert_eq!(cfg.port, 25);
    }

    #[test]
    fn apply_answers_upgrade_preserves_unspecified_fields() {
        use bindings::exports::greentic::component::qa::Guest as QaGuest;
        use bindings::exports::greentic::component::qa::Mode;
        let answers = json!({
            "existing_config": {
                "enabled": true,
                "public_base_url": "https://example.com",
                "host": "smtp.example.com",
                "port": 587,
                "username": "user-a",
                "from_address": "from@example.com",
                "tls_mode": "starttls",
                "default_to_address": "old@example.com",
                "password": "secret-a"
            },
            "default_to_address": "new@example.com"
        });
        let out =
            <Component as QaGuest>::apply_answers(Mode::Upgrade, canonical_cbor_bytes(&answers));
        let out_json: Value = decode_cbor(&out).expect("decode apply output");
        assert_eq!(out_json.get("ok"), Some(&Value::Bool(true)));
        let config = out_json.get("config").expect("config object");
        assert_eq!(
            config.get("host"),
            Some(&Value::String("smtp.example.com".to_string()))
        );
        assert_eq!(
            config.get("default_to_address"),
            Some(&Value::String("new@example.com".to_string()))
        );
    }

}
