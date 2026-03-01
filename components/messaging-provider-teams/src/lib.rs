//! Teams messaging provider component.
//!
//! Uses Bot Framework / Bot Service for messaging instead of Microsoft Graph API.
//!
//! Implementation details are split across submodules:
//! - `auth`: Bot Framework authentication (token acquisition, JWT validation)
//! - `config`: Configuration parsing and validation
//! - `describe`: Provider description and QA specs
//! - `ops`: Core operations (send, reply, render_plan, encode, send_payload)

use provider_common::component_v0_6::{canonical_cbor_bytes, decode_cbor};
use provider_common::helpers::json_bytes;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

mod bindings {
    wit_bindgen::generate!({
        path: "wit/messaging-provider-teams",
        world: "component-v0-v6-v0",
        generate_all
    });
}

pub(crate) mod auth;
pub(crate) mod config;
mod describe;
mod ops;

// Provider identification
pub(crate) const PROVIDER_ID: &str = "messaging-provider-teams";
pub(crate) const PROVIDER_TYPE: &str = "messaging.teams.bot";
pub(crate) const WORLD_ID: &str = "component-v0-v6-v0";

// Bot Service constants
pub(crate) const DEFAULT_BOT_APP_ID_KEY: &str = "MS_BOT_APP_ID";
pub(crate) const DEFAULT_BOT_APP_PASSWORD_KEY: &str = "MS_BOT_APP_PASSWORD";
pub(crate) const DEFAULT_BOT_TOKEN_ENDPOINT: &str =
    "https://login.microsoftonline.com/f28345e6-e7f5-4fff-b9b2-16baf26c13b4/oauth2/v2.0/token";
pub(crate) const DEFAULT_BOT_TOKEN_SCOPE: &str = "https://api.botframework.com/.default";

use config::{ProviderConfigOut, default_config_out, validate_config_out};
use describe::{
    DEFAULT_KEYS, I18N_KEYS, I18N_PAIRS, SETUP_QUESTIONS, build_describe_payload, build_qa_spec,
};
use ops::{encode_op, handle_reply, handle_send, ingest_http, render_plan, send_payload};

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
            "teams",
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

// ============================================================================
// Dispatch
// ============================================================================

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
        "reply" => handle_reply(input_json),
        "ingest_http" => ingest_http(input_json),
        "render_plan" => render_plan(input_json),
        "encode" => encode_op(input_json),
        "send_payload" => send_payload(input_json),
        // Note: subscription_* operations removed - Bot Service handles subscriptions automatically
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
        merged.ms_bot_app_id =
            string_or_default(&answers, "ms_bot_app_id", &merged.ms_bot_app_id);
        merged.ms_bot_app_password =
            optional_string_from(&answers, "ms_bot_app_password").or(merged.ms_bot_app_password.clone());
        merged.default_service_url =
            optional_string_from(&answers, "default_service_url").or(merged.default_service_url.clone());
        merged.team_id = optional_string_from(&answers, "team_id").or(merged.team_id.clone());
        merged.channel_id =
            optional_string_from(&answers, "channel_id").or(merged.channel_id.clone());
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
        if has("ms_bot_app_id") {
            merged.ms_bot_app_id =
                string_or_default(&answers, "ms_bot_app_id", &merged.ms_bot_app_id);
        }
        if has("ms_bot_app_password") {
            merged.ms_bot_app_password = optional_string_from(&answers, "ms_bot_app_password");
        }
        if has("default_service_url") {
            merged.default_service_url = optional_string_from(&answers, "default_service_url");
        }
        if has("team_id") {
            merged.team_id = optional_string_from(&answers, "team_id");
        }
        if has("channel_id") {
            merged.channel_id = optional_string_from(&answers, "channel_id");
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
    use provider_common::component_v0_6::schema_hash;
    use std::collections::BTreeSet;

    #[test]
    fn parse_config_requires_new_fields() {
        let cfg = br#"{"enabled":true,"ms_bot_app_id":"app-id","public_base_url":"https://example.com"}"#;
        let parsed = parse_config_bytes(cfg).expect("valid config");
        assert!(parsed.enabled);
        assert_eq!(parsed.ms_bot_app_id, "app-id");
    }

    #[test]
    fn load_config_prefers_nested_config() {
        let input = json!({
            "config": {
                "enabled": true,
                "ms_bot_app_id": "app-id",
                "public_base_url": "https://example.com",
                "team_id": "team-abc"
            },
        });
        let cfg = config::load_config(&input).expect("config");
        assert_eq!(cfg.team_id.as_deref(), Some("team-abc"));
    }

    #[test]
    fn parse_config_rejects_unknown() {
        let cfg = br#"{"enabled":true,"ms_bot_app_id":"app-id","public_base_url":"https://example.com","unknown":"field"}"#;
        let err = parse_config_bytes(cfg).unwrap_err();
        assert!(err.contains("unknown field"));
    }

    #[test]
    fn schema_hash_is_stable() {
        let describe = build_describe_payload();
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
    fn describe_passes_strict_rules() {
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
        assert_eq!(keys, vec!["ms_bot_app_id", "public_base_url"]);
    }

    #[test]
    fn apply_answers_upgrade_preserves_unspecified_fields() {
        use bindings::exports::greentic::component::qa::Guest as QaGuest;
        use bindings::exports::greentic::component::qa::Mode;
        let answers = json!({
            "existing_config": {
                "enabled": true,
                "ms_bot_app_id": "app-id",
                "public_base_url": "https://example.com",
                "team_id": "team-123",
                "ms_bot_app_password": "secret-a"
            },
            "team_id": "team-456"
        });
        let out =
            <Component as QaGuest>::apply_answers(Mode::Upgrade, canonical_cbor_bytes(&answers));
        let out_json: Value = decode_cbor(&out).expect("decode apply output");
        assert_eq!(out_json.get("ok"), Some(&Value::Bool(true)));
        let config = out_json.get("config").expect("config object");
        assert_eq!(
            config.get("ms_bot_app_password"),
            Some(&Value::String("secret-a".to_string()))
        );
        assert_eq!(
            config.get("team_id"),
            Some(&Value::String("team-456".to_string()))
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
            "ms_bot_app_id": "app-id",
            "public_base_url": "not-a-url"
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
