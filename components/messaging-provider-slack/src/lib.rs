use config::{ProviderConfigOut, default_config_out, validate_config_out};
use describe::{build_describe_payload, build_qa_spec, I18N_KEYS};
use ops::{encode_op, handle_send, ingest_http, render_plan, send_payload};
use provider_common::component_v0_6::{canonical_cbor_bytes, decode_cbor};
use provider_common::helpers::{
    cbor_json_invoke_bridge, existing_config_from_answers, json_bytes, optional_string_from,
    schema_core_describe, schema_core_healthcheck, schema_core_validate_config, string_or_default,
};
use provider_common::qa_helpers::ApplyAnswersResult;
use serde_json::{Value, json};

mod bindings {
    wit_bindgen::generate!({
        path: "wit/messaging-provider-slack",
        world: "component-v0-v6-v0",
        generate_all
    });
}

mod config;
mod describe;
mod ops;

const PROVIDER_ID: &str = "messaging-provider-slack";
const PROVIDER_TYPE: &str = "messaging.slack.api";
const WORLD_ID: &str = "component-v0-v6-v0";
const DEFAULT_API_BASE: &str = "https://slack.com/api";
const DEFAULT_BOT_TOKEN_KEY: &str = "SLACK_BOT_TOKEN";

struct Component;

impl bindings::exports::greentic::component::descriptor::Guest for Component {
    fn describe() -> Vec<u8> {
        canonical_cbor_bytes(&build_describe_payload())
    }
}

impl bindings::exports::greentic::component::runtime::Guest for Component {
    fn invoke(op: String, input_cbor: Vec<u8>) -> Vec<u8> {
        cbor_json_invoke_bridge(&op, &input_cbor, Some("send"), |op, input| match op {
            "send" => handle_send(input, false),
            "reply" => handle_send(input, true),
            "ingest_http" => ingest_http(input),
            "render_plan" => render_plan(input),
            "encode" => encode_op(input),
            "send_payload" => send_payload(input),
            other => {
                json_bytes(&json!({"ok": false, "error": format!("unsupported op: {other}")}))
            }
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
            merged.default_channel = optional_string_from(&answers, "default_channel")
                .or(merged.default_channel.clone());
            merged.public_base_url =
                string_or_default(&answers, "public_base_url", &merged.public_base_url);
            merged.api_base_url = string_or_default(&answers, "api_base_url", &merged.api_base_url);
            if merged.api_base_url.trim().is_empty() {
                merged.api_base_url = DEFAULT_API_BASE.to_string();
            }
            merged.bot_token = string_or_default(&answers, "bot_token", &merged.bot_token);
        }

        if mode == bindings::exports::greentic::component::qa::Mode::Upgrade {
            if has("enabled") {
                merged.enabled = answers
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(merged.enabled);
            }
            if has("default_channel") {
                merged.default_channel = optional_string_from(&answers, "default_channel");
            }
            if has("public_base_url") {
                merged.public_base_url =
                    string_or_default(&answers, "public_base_url", &merged.public_base_url);
            }
            if has("api_base_url") {
                merged.api_base_url =
                    string_or_default(&answers, "api_base_url", &merged.api_base_url);
            }
            if has("bot_token") {
                merged.bot_token = string_or_default(&answers, "bot_token", &merged.bot_token);
            }
            if merged.api_base_url.trim().is_empty() {
                merged.api_base_url = DEFAULT_API_BASE.to_string();
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
        let op = if op == "run" { "send" } else { op.as_str() };
        match op {
            "send" => handle_send(&input_json, false),
            "reply" => handle_send(&input_json, true),
            "ingest_http" => ingest_http(&input_json),
            "render_plan" => render_plan(&input_json),
            "encode" => encode_op(&input_json),
            "send_payload" => send_payload(&input_json),
            other => {
                json_bytes(&json!({"ok": false, "error": format!("unsupported op: {other}")}))
            }
        }
    }
}

bindings::export!(Component with_types_in bindings);

#[cfg(test)]
mod tests {
    use super::*;

    provider_common::standard_provider_tests! {
        describe_fn: build_describe_payload,
        qa_spec_fn: build_qa_spec,
        i18n_keys: I18N_KEYS,
        world_id: WORLD_ID,
        provider_id: PROVIDER_ID,
        schema_hash: "0d7cbda46632fd39f7ade4774c1dee9a7deebd7b382b5c785a384b1899faa519",
        qa_default_keys: ["public_base_url", "bot_token"],
        mode_type: bindings::exports::greentic::component::qa::Mode,
        component_type: Component,
        qa_guest_path: bindings::exports::greentic::component::qa::Guest,
        validation_answers: {"public_base_url": "not-a-url", "bot_token": "token-a"},
        validation_field: "public_base_url",
    }

    #[test]
    fn parse_config_rejects_unknown() {
        let cfg = br#"{"enabled":true,"public_base_url":"https://x","api_base_url":"https://slack.com/api","bot_token":"x","unknown":true}"#;
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
                "default_channel": "C1",
                "public_base_url": "https://example.com",
                "api_base_url": "https://slack.com/api",
                "bot_token": "token-a"
            },
            "default_channel": "C2"
        });
        let bytes = canonical_cbor_bytes(&answers);
        let out = <Component as QaGuest>::apply_answers(Mode::Upgrade, bytes);
        let out_json: Value = decode_cbor(&out).expect("decode apply output");
        assert_eq!(out_json.get("ok"), Some(&Value::Bool(true)));
        let config = out_json.get("config").expect("config object");
        assert_eq!(
            config.get("public_base_url"),
            Some(&Value::String("https://example.com".to_string()))
        );
        assert_eq!(
            config.get("bot_token"),
            Some(&Value::String("token-a".to_string()))
        );
        assert_eq!(
            config.get("default_channel"),
            Some(&Value::String("C2".to_string()))
        );
    }
}
