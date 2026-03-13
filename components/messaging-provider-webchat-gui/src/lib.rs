//! WebChat GUI messaging provider component.

use provider_common::component_v0_6::{canonical_cbor_bytes, decode_cbor};
use provider_common::helpers::json_bytes;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

mod bindings {
    wit_bindgen::generate!({
        path: "../messaging-provider-webchat/wit/messaging-provider-webchat",
        world: "component-v0-v6-v0",
        generate_all
    });
}

#[path = "../../messaging-provider-webchat/src/config.rs"]
pub(crate) mod config;
#[path = "../../messaging-provider-webchat/src/describe.rs"]
mod describe;
pub(crate) mod directline;
#[path = "../../messaging-provider-webchat/src/ops.rs"]
mod ops;

pub(crate) const PROVIDER_ID: &str = "messaging-provider-webchat-gui";
pub(crate) const PROVIDER_TYPE: &str = "messaging.webchat-gui";
pub(crate) const WORLD_ID: &str = "component-v0-v6-v0";

use config::{ProviderConfigOut, default_config_out, default_mode, validate_config_out};
use describe::{
    DEFAULT_KEYS, I18N_KEYS, I18N_PAIRS, SETUP_QUESTIONS, build_describe_payload, build_qa_spec,
};
use ops::{encode_op, handle_ingest, handle_send, ingest_http, render_plan, send_payload};

struct Component;

impl bindings::exports::greentic::component::descriptor::Guest for Component {
    fn describe() -> Vec<u8> {
        canonical_cbor_bytes(&build_describe_payload())
    }
}

impl bindings::exports::greentic::component::runtime::Guest for Component {
    fn invoke(op: String, input_cbor: Vec<u8>) -> Vec<u8> {
        let input_value: Value = match decode_cbor(&input_cbor) {
            Ok(value) => value,
            Err(err) => {
                return canonical_cbor_bytes(
                    &json!({"ok": false, "error": format!("invalid input cbor: {err}")}),
                );
            }
        };
        let input_json = serde_json::to_vec(&input_value).unwrap_or_default();
        let output_json = dispatch_json_invoke(&op, &input_json);
        let output_value: Value = serde_json::from_slice(&output_json)
            .unwrap_or_else(|_| json!({"ok": false, "error": "provider produced invalid json"}));
        canonical_cbor_bytes(&output_value)
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
        apply_answers_impl(mode, answers_cbor)
    }
}

impl bindings::exports::greentic::component::component_i18n::Guest for Component {
    fn i18n_keys() -> Vec<String> {
        I18N_KEYS.iter().map(|k| (*k).to_string()).collect()
    }

    fn i18n_bundle(locale: String) -> Vec<u8> {
        describe::i18n_bundle(locale)
    }
}

impl bindings::exports::greentic::provider_schema_core::schema_core_api::Guest for Component {
    fn describe() -> Vec<u8> {
        serde_json::to_vec(&build_describe_payload()).unwrap_or_default()
    }

    fn validate_config(_config_json: Vec<u8>) -> Vec<u8> {
        json_bytes(&json!({"ok": true}))
    }

    fn healthcheck() -> Vec<u8> {
        json_bytes(&json!({"status": "healthy"}))
    }

    fn invoke(op: String, input_json: Vec<u8>) -> Vec<u8> {
        if let Some(result) = provider_common::qa_invoke_bridge::dispatch_qa_ops_with_i18n(
            &op,
            &input_json,
            "webchat",
            SETUP_QUESTIONS,
            DEFAULT_KEYS,
            I18N_KEYS,
            I18N_PAIRS,
            apply_answers_bridge,
        ) {
            return result;
        }
        dispatch_json_invoke(&op, &input_json)
    }
}

bindings::export!(Component with_types_in bindings);

fn apply_answers_bridge(mode: &str, answers_cbor: Vec<u8>) -> Vec<u8> {
    use bindings::exports::greentic::component::qa::Mode;
    let mode = match mode {
        "setup" => Mode::Setup,
        "upgrade" => Mode::Upgrade,
        "remove" => Mode::Remove,
        _ => Mode::Default,
    };
    apply_answers_impl(mode, answers_cbor)
}

fn dispatch_json_invoke(op: &str, input_json: &[u8]) -> Vec<u8> {
    match op {
        "run" | "send" => handle_send(input_json),
        "ingest" => handle_ingest(input_json),
        "ingest_http" => ingest_http(input_json),
        "render_plan" => render_plan(input_json),
        "encode" => encode_op(input_json),
        "send_payload" => send_payload(input_json),
        other => json_bytes(&json!({"ok": false, "error": format!("unsupported op: {other}")})),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ApplyAnswersResult {
    ok: bool,
    config: Option<ProviderConfigOut>,
    remove: Option<RemovePlan>,
    diagnostics: Vec<String>,
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RemovePlan {
    remove_all: bool,
    cleanup: Vec<String>,
}

fn apply_answers_impl(
    mode: bindings::exports::greentic::component::qa::Mode,
    answers_cbor: Vec<u8>,
) -> Vec<u8> {
    use bindings::exports::greentic::component::qa::Mode;

    let answers: Value = match decode_cbor(&answers_cbor) {
        Ok(value) => value,
        Err(err) => {
            return canonical_cbor_bytes(&ApplyAnswersResult {
                ok: false,
                config: None,
                remove: None,
                diagnostics: Vec::new(),
                error: Some(format!("invalid answers cbor: {err}")),
            });
        }
    };

    if mode == Mode::Remove {
        return canonical_cbor_bytes(&ApplyAnswersResult {
            ok: true,
            config: None,
            remove: Some(RemovePlan {
                remove_all: true,
                cleanup: vec![
                    "delete_config_key".to_string(),
                    "delete_provenance_key".to_string(),
                    "delete_provider_state_namespace".to_string(),
                ],
            }),
            diagnostics: Vec::new(),
            error: None,
        });
    }

    let mut merged = existing_config_from_answers(&answers).unwrap_or_else(default_config_out);
    let answer_obj = answers.as_object();
    let has = |key: &str| answer_obj.is_some_and(|obj| obj.contains_key(key));

    if mode == Mode::Setup || mode == Mode::Default {
        merged.enabled = answers
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(merged.enabled);
        merged.public_base_url =
            string_or_default(&answers, "public_base_url", &merged.public_base_url);
        merged.mode = string_or_default(&answers, "mode", &merged.mode);
        if merged.mode.trim().is_empty() {
            merged.mode = default_mode();
        }
        merged.route = optional_string_from(&answers, "route").or(merged.route.clone());
        merged.tenant_channel_id = optional_string_from(&answers, "tenant_channel_id")
            .or(merged.tenant_channel_id.clone());
        merged.base_url = optional_string_from(&answers, "base_url").or(merged.base_url.clone());
    }

    if mode == Mode::Upgrade {
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
        if has("mode") {
            merged.mode = string_or_default(&answers, "mode", &merged.mode);
        }
        if has("route") {
            merged.route = optional_string_from(&answers, "route");
        }
        if has("tenant_channel_id") {
            merged.tenant_channel_id = optional_string_from(&answers, "tenant_channel_id");
        }
        if has("base_url") {
            merged.base_url = optional_string_from(&answers, "base_url");
        }
    }

    if let Err(error) = validate_config_out(&merged) {
        return canonical_cbor_bytes(&ApplyAnswersResult {
            ok: false,
            config: None,
            remove: None,
            diagnostics: Vec::new(),
            error: Some(error),
        });
    }

    canonical_cbor_bytes(&ApplyAnswersResult {
        ok: true,
        config: Some(merged),
        remove: None,
        diagnostics: Vec::new(),
        error: None,
    })
}

fn existing_config_from_answers(answers: &Value) -> Option<ProviderConfigOut> {
    answers
        .get("existing_config")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
}

fn string_or_default(answers: &Value, key: &str, fallback: &str) -> String {
    answers
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| fallback.to_string())
}

fn optional_string_from(answers: &Value, key: &str) -> Option<String> {
    answers
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}
