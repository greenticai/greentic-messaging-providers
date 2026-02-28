//! Email messaging provider component.
//!
//! This module provides the WASM component interface for the email provider.
//! Implementation details are split across submodules:
//! - `config`: Configuration parsing and validation
//! - `auth`: OAuth/Graph API authentication
//! - `describe`: Provider description and QA specs
//! - `ops`: Core operations (send, reply, render_plan, encode, send_payload)
//! - `graph`: Microsoft Graph API helpers and subscriptions
//! - `ingress`: HTTP ingress handling

use provider_common::component_v0_6::{canonical_cbor_bytes, decode_cbor};
use provider_common::helpers::json_bytes;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

mod bindings {
    wit_bindgen::generate!({
        path: "wit/messaging-provider-email",
        world: "component-v0-v6-v0",
        generate_all
    });
}

pub(crate) mod auth;
mod config;
mod describe;
mod graph;
mod ingress;
mod ops;

pub(crate) const PROVIDER_ID: &str = "messaging-provider-email";
pub(crate) const PROVIDER_TYPE: &str = "messaging.email.smtp";
pub(crate) const WORLD_ID: &str = "component-v0-v6-v0";
pub(crate) const DEFAULT_GRAPH_BASE: &str = "https://graph.microsoft.com/v1.0";
pub(crate) const GRAPH_MAX_EXPIRATION_MINUTES: u32 = 4230;

use config::{ProviderConfigOut, default_config_out, validate_config_out};
use describe::{I18N_KEYS, build_describe_payload, build_qa_spec};
use graph::{subscription_delete, subscription_ensure, subscription_renew};
use ingress::ingest_http;
use ops::{encode_op, handle_reply, handle_send, render_plan, send_payload};

// ============================================================================
// Component trait implementations
// ============================================================================

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
        let op = if op == "run" { "send".to_string() } else { op };
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
        let op = if op == "run" { "send".to_string() } else { op };
        dispatch_json_invoke(&op, &input_json)
    }
}

bindings::export!(Component with_types_in bindings);

// ============================================================================
// Dispatch
// ============================================================================

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

// ============================================================================
// QA apply_answers implementation
// ============================================================================

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
                    "best_effort_revoke_tokens".to_string(),
                    "best_effort_delete_provider_owned_secrets".to_string(),
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
        merged.host = string_or_default(&answers, "host", &merged.host);
        merged.port = answers
            .get("port")
            .and_then(Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .unwrap_or(merged.port);
        merged.username = string_or_default(&answers, "username", &merged.username);
        merged.from_address = string_or_default(&answers, "from_address", &merged.from_address);
        merged.tls_mode = string_or_default(&answers, "tls_mode", &merged.tls_mode);
        merged.default_to_address =
            optional_string_from(&answers, "default_to_address").or(merged.default_to_address);
        merged.password = optional_string_from(&answers, "password").or(merged.password);
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
        .or_else(|| answers.get("config").cloned())
        .and_then(|value| serde_json::from_value::<ProviderConfigOut>(value).ok())
}

fn optional_string_from(answers: &Value, key: &str) -> Option<String> {
    let value = answers.get(key)?;
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Value::Null => None,
        _ => None,
    }
}

fn string_or_default(answers: &Value, key: &str, default: &str) -> String {
    answers
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| default.to_string())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use config::parse_config_bytes;
    use std::collections::BTreeSet;

    #[test]
    fn parse_config_requires_new_fields() {
        let cfg = br#"{"enabled":true,"public_base_url":"https://example.com","host":"smtp.example.com","port":587,"username":"u","from_address":"from@example.com","tls_mode":"starttls"}"#;
        let parsed = parse_config_bytes(cfg).expect("valid config");
        assert!(parsed.enabled);
    }

    #[test]
    fn parse_config_rejects_unknown() {
        let cfg = br#"{"enabled":true,"public_base_url":"https://example.com","host":"smtp","port":587,"username":"u","from_address":"f","tls_mode":"starttls","unknown":true}"#;
        let err = parse_config_bytes(cfg).unwrap_err();
        assert!(err.contains("unknown field"));
    }

    #[test]
    fn load_config_prefers_nested() {
        use config::load_config;
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
        let cfg = load_config(&input).unwrap();
        assert_eq!(cfg.host, "a");
        assert_eq!(cfg.port, 25);
    }

    #[test]
    fn schema_hash_is_stable() {
        let describe = build_describe_payload();
        assert_eq!(
            describe.schema_hash,
            "a022076adb33dab084ad655fb83b4857a9d4aa7fd81b1d4d694a509789a63890"
        );
    }

    #[test]
    fn describe_passes_strict_rules() {
        use provider_common::component_v0_6::schema_hash;
        let describe = build_describe_payload();
        assert!(!describe.operations.is_empty());
        assert_eq!(
            describe.schema_hash,
            schema_hash(
                &describe.input_schema,
                &describe.output_schema,
                &describe.config_schema
            )
        );
    }

    #[test]
    fn i18n_keys_cover_qa_specs() {
        use bindings::exports::greentic::component::qa::Mode;

        let keyset = I18N_KEYS
            .iter()
            .map(|value| (*value).to_string())
            .collect::<BTreeSet<_>>();

        for mode in [Mode::Default, Mode::Setup, Mode::Upgrade, Mode::Remove] {
            let spec = build_qa_spec(mode);
            assert!(keyset.contains(&spec.title.key));
            for question in spec.questions {
                assert!(keyset.contains(&question.label.key));
            }
        }
    }

    #[test]
    fn qa_default_asks_required_minimum() {
        use bindings::exports::greentic::component::qa::Mode;
        let spec = build_qa_spec(Mode::Default);
        let keys = spec
            .questions
            .into_iter()
            .map(|question| question.id)
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            vec!["public_base_url", "host", "username", "from_address"]
        );
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

    #[test]
    fn apply_answers_remove_returns_cleanup_plan() {
        use bindings::exports::greentic::component::qa::Guest as QaGuest;
        use bindings::exports::greentic::component::qa::Mode;
        let out =
            <Component as QaGuest>::apply_answers(Mode::Remove, canonical_cbor_bytes(&json!({})));
        let out_json: Value = decode_cbor(&out).expect("decode apply output");
        assert_eq!(out_json.get("ok"), Some(&Value::Bool(true)));
        assert_eq!(out_json.get("config"), Some(&Value::Null));
        let cleanup = out_json
            .get("remove")
            .and_then(|value| value.get("cleanup"))
            .and_then(Value::as_array)
            .expect("cleanup steps");
        assert!(!cleanup.is_empty());
    }

    #[test]
    fn apply_answers_validates_public_base_url() {
        use bindings::exports::greentic::component::qa::Guest as QaGuest;
        use bindings::exports::greentic::component::qa::Mode;
        let answers = json!({
            "public_base_url": "not-a-url",
            "host": "smtp.example.com",
            "username": "user-a",
            "from_address": "from@example.com"
        });
        let out =
            <Component as QaGuest>::apply_answers(Mode::Default, canonical_cbor_bytes(&answers));
        let out_json: Value = decode_cbor(&out).expect("decode apply output");
        assert_eq!(out_json.get("ok"), Some(&Value::Bool(false)));
        let error = out_json
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(error.contains("public_base_url"));
    }
}
