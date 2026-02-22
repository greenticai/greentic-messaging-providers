use provider_common::component_v0_6::{
    DescribePayload, OperationDescriptor, RedactionRule, SchemaIr, canonical_cbor_bytes,
    decode_cbor, schema_hash,
};
use provider_common::helpers::{existing_config_from_answers, i18n, string_or_default};
use provider_common::qa_helpers::ApplyAnswersResult;
use serde::{Deserialize, Serialize};

mod bindings {
    wit_bindgen::generate!({
        path: "wit/messaging-provider-dummy",
        world: "component-v0-v6-v0",
        generate_all
    });
}

const PROVIDER_ID: &str = "messaging-provider-dummy";
const WORLD_ID: &str = "component-v0-v6-v0";

const I18N_KEYS: &[&str] = &[
    "dummy.op.run.title",
    "dummy.op.run.description",
    "dummy.schema.input.title",
    "dummy.schema.input.description",
    "dummy.schema.input.message.title",
    "dummy.schema.input.message.description",
    "dummy.schema.output.title",
    "dummy.schema.output.description",
    "dummy.schema.output.ok.title",
    "dummy.schema.output.ok.description",
    "dummy.schema.output.message_id.title",
    "dummy.schema.output.message_id.description",
    "dummy.schema.config.title",
    "dummy.schema.config.description",
    "dummy.schema.config.enabled.title",
    "dummy.schema.config.enabled.description",
    "dummy.schema.config.api_token.title",
    "dummy.schema.config.api_token.description",
    "dummy.schema.config.endpoint_url.title",
    "dummy.schema.config.endpoint_url.description",
    "dummy.qa.default.title",
    "dummy.qa.setup.title",
    "dummy.qa.upgrade.title",
    "dummy.qa.remove.title",
    "dummy.qa.setup.enabled",
    "dummy.qa.setup.api_token",
    "dummy.qa.setup.endpoint_url",
];

struct Component;

impl bindings::exports::greentic::component::descriptor::Guest for Component {
    fn describe() -> Vec<u8> {
        canonical_cbor_bytes(&build_describe_payload())
    }
}

impl bindings::exports::greentic::component::runtime::Guest for Component {
    fn invoke(op: String, input_cbor: Vec<u8>) -> Vec<u8> {
        if op != "run" {
            return canonical_cbor_bytes(&RunResult {
                ok: false,
                message_id: None,
                error: Some(format!("unsupported op: {op}")),
            });
        }

        let input: RunInput = match decode_cbor(&input_cbor) {
            Ok(value) => value,
            Err(err) => {
                return canonical_cbor_bytes(&RunResult {
                    ok: false,
                    message_id: None,
                    error: Some(format!("invalid input cbor: {err}")),
                });
            }
        };

        let message_id = format!(
            "dummy:{}",
            provider_common::component_v0_6::sha256_hex(input.message.as_bytes())
        );

        canonical_cbor_bytes(&RunResult {
            ok: true,
            message_id: Some(message_id),
            error: None,
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
        let answers: serde_json::Value = match decode_cbor(&answers_cbor) {
            Ok(value) => value,
            Err(err) => {
                return canonical_cbor_bytes(&ApplyAnswersResult::<ProviderConfig>::decode_error(
                    format!("invalid answers cbor: {err}"),
                ));
            }
        };

        if mode == bindings::exports::greentic::component::qa::Mode::Remove {
            return canonical_cbor_bytes(
                &ApplyAnswersResult::<ProviderConfig>::remove(vec![
                    "delete_config_key".to_string(),
                    "delete_provenance_key".to_string(),
                    "delete_provider_state_namespace".to_string(),
                ]),
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
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(merged.enabled);
            merged.api_token = string_or_default(&answers, "api_token", &merged.api_token);
            merged.endpoint_url = string_or_default(&answers, "endpoint_url", &merged.endpoint_url);
        }

        if mode == bindings::exports::greentic::component::qa::Mode::Upgrade {
            if has("enabled") {
                merged.enabled = answers
                    .get("enabled")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(merged.enabled);
            }
            if has("api_token") {
                merged.api_token = string_or_default(&answers, "api_token", &merged.api_token);
            }
            if has("endpoint_url") {
                merged.endpoint_url =
                    string_or_default(&answers, "endpoint_url", &merged.endpoint_url);
            }
        }

        if let Err(error) = validate_config_out(&merged) {
            return canonical_cbor_bytes(
                &ApplyAnswersResult::<ProviderConfig>::validation_error(error),
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
        provider_common::helpers::i18n_bundle_from_pairs(locale, &[
            ("dummy.op.run.title", "Run"),
            ("dummy.op.run.description", "Run dummy provider operation"),
            ("dummy.schema.input.title", "Dummy input"),
            ("dummy.schema.input.description", "Input for dummy provider run"),
            ("dummy.schema.input.message.title", "Message"),
            ("dummy.schema.input.message.description", "Message text to hash"),
            ("dummy.schema.output.title", "Dummy output"),
            ("dummy.schema.output.description", "Result of dummy provider run"),
            ("dummy.schema.output.ok.title", "Success"),
            ("dummy.schema.output.ok.description", "Whether provider run succeeded"),
            ("dummy.schema.output.message_id.title", "Message ID"),
            ("dummy.schema.output.message_id.description", "Deterministic message identifier"),
            ("dummy.schema.config.title", "Dummy config"),
            ("dummy.schema.config.description", "Dummy provider configuration"),
            ("dummy.schema.config.enabled.title", "Enabled"),
            ("dummy.schema.config.enabled.description", "Enable this provider"),
            ("dummy.schema.config.api_token.title", "API token"),
            ("dummy.schema.config.api_token.description", "Tenant token for dummy provider"),
            ("dummy.schema.config.endpoint_url.title", "Endpoint URL"),
            ("dummy.schema.config.endpoint_url.description", "Dummy endpoint URL"),
            ("dummy.qa.default.title", "Default"),
            ("dummy.qa.setup.title", "Setup"),
            ("dummy.qa.upgrade.title", "Upgrade"),
            ("dummy.qa.remove.title", "Remove"),
            ("dummy.qa.setup.enabled", "Enable provider"),
            ("dummy.qa.setup.api_token", "API token"),
            ("dummy.qa.setup.endpoint_url", "Endpoint URL"),
        ])
    }
}

// Backward-compatible schema-core-api export for operator v0.4.x
impl bindings::exports::greentic::provider_schema_core::schema_core_api::Guest for Component {
    fn describe() -> Vec<u8> {
        provider_common::helpers::schema_core_describe(&build_describe_payload())
    }

    fn validate_config(_config_json: Vec<u8>) -> Vec<u8> {
        provider_common::helpers::schema_core_validate_config()
    }

    fn healthcheck() -> Vec<u8> {
        provider_common::helpers::schema_core_healthcheck()
    }

    fn invoke(op: String, input_json: Vec<u8>) -> Vec<u8> {
        if op != "run" {
            return serde_json::to_vec(&RunResult {
                ok: false,
                message_id: None,
                error: Some(format!("unsupported op: {op}")),
            })
            .unwrap_or_default();
        }
        let input: RunInput = match serde_json::from_slice(&input_json) {
            Ok(value) => value,
            Err(err) => {
                return serde_json::to_vec(&RunResult {
                    ok: false,
                    message_id: None,
                    error: Some(format!("invalid input json: {err}")),
                })
                .unwrap_or_default();
            }
        };
        let message_id = format!(
            "dummy:{}",
            provider_common::component_v0_6::sha256_hex(input.message.as_bytes())
        );
        serde_json::to_vec(&RunResult {
            ok: true,
            message_id: Some(message_id),
            error: None,
        })
        .unwrap_or_default()
    }
}

bindings::export!(Component with_types_in bindings);

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RunInput {
    message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RunResult {
    ok: bool,
    message_id: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProviderConfig {
    enabled: bool,
    api_token: String,
    endpoint_url: String,
}

fn build_describe_payload() -> DescribePayload {
    let input_schema = input_schema();
    let output_schema = output_schema();
    let config_schema = config_schema();
    let hash = schema_hash(&input_schema, &output_schema, &config_schema);

    DescribePayload {
        provider: PROVIDER_ID.to_string(),
        world: WORLD_ID.to_string(),
        operations: vec![OperationDescriptor {
            name: "run".to_string(),
            title: i18n("dummy.op.run.title"),
            description: i18n("dummy.op.run.description"),
        }],
        input_schema,
        output_schema,
        config_schema,
        redactions: vec![RedactionRule {
            path: "$.api_token".to_string(),
            strategy: "replace".to_string(),
        }],
        schema_hash: hash,
    }
}

const SETUP_QUESTIONS: &[provider_common::helpers::QaQuestionDef] = &[
    ("enabled", "dummy.qa.setup.enabled", true),
    ("api_token", "dummy.qa.setup.api_token", true),
    ("endpoint_url", "dummy.qa.setup.endpoint_url", true),
];
const DEFAULT_KEYS: &[&str] = &["api_token", "endpoint_url"];

fn build_qa_spec(mode: bindings::exports::greentic::component::qa::Mode) -> provider_common::component_v0_6::QaSpec {
    use bindings::exports::greentic::component::qa::Mode;
    let mode_str = match mode {
        Mode::Default => "default",
        Mode::Setup => "setup",
        Mode::Upgrade => "upgrade",
        Mode::Remove => "remove",
    };
    provider_common::helpers::qa_spec_for_mode(mode_str, "dummy", SETUP_QUESTIONS, DEFAULT_KEYS)
}

fn input_schema() -> SchemaIr {
    provider_common::helpers::schema_obj(
        "dummy.schema.input.title", "dummy.schema.input.description",
        vec![("message", true, provider_common::helpers::schema_str("dummy.schema.input.message.title", "dummy.schema.input.message.description"))],
        false,
    )
}

fn output_schema() -> SchemaIr {
    provider_common::helpers::schema_obj(
        "dummy.schema.output.title", "dummy.schema.output.description",
        vec![
            ("ok", true, provider_common::helpers::schema_bool_ir("dummy.schema.output.ok.title", "dummy.schema.output.ok.description")),
            ("message_id", false, provider_common::helpers::schema_str("dummy.schema.output.message_id.title", "dummy.schema.output.message_id.description")),
        ],
        false,
    )
}

fn config_schema() -> SchemaIr {
    provider_common::helpers::schema_obj(
        "dummy.schema.config.title", "dummy.schema.config.description",
        vec![
            ("enabled", true, provider_common::helpers::schema_bool_ir("dummy.schema.config.enabled.title", "dummy.schema.config.enabled.description")),
            ("api_token", true, provider_common::helpers::schema_secret("dummy.schema.config.api_token.title", "dummy.schema.config.api_token.description")),
            ("endpoint_url", true, provider_common::helpers::schema_str_fmt("dummy.schema.config.endpoint_url.title", "dummy.schema.config.endpoint_url.description", "uri")),
        ],
        false,
    )
}

fn default_config_out() -> ProviderConfig {
    ProviderConfig {
        enabled: true,
        api_token: String::new(),
        endpoint_url: String::new(),
    }
}

fn validate_config_out(config: &ProviderConfig) -> Result<(), String> {
    if config.api_token.trim().is_empty() {
        return Err("config validation failed: api_token is required".to_string());
    }
    if config.endpoint_url.trim().is_empty() {
        return Err("config validation failed: endpoint_url is required".to_string());
    }
    if !(config.endpoint_url.starts_with("http://") || config.endpoint_url.starts_with("https://"))
    {
        return Err("config validation failed: endpoint_url must be an absolute URL".to_string());
    }
    Ok(())
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
        schema_hash: "4092e7c953666fbd6a2c172edc5a333e5d0a00f4f4bb173bc052ea270ef70dd5",
        qa_default_keys: ["api_token", "endpoint_url"],
        mode_type: bindings::exports::greentic::component::qa::Mode,
        component_type: Component,
        qa_guest_path: bindings::exports::greentic::component::qa::Guest,
        validation_answers: {"api_token": "token-a", "endpoint_url": "not-a-url"},
        validation_field: "endpoint_url",
    }

    #[test]
    fn apply_answers_upgrade_preserves_unspecified_fields() {
        use bindings::exports::greentic::component::qa::Guest as QaGuest;
        use bindings::exports::greentic::component::qa::Mode;
        let answers = serde_json::json!({
            "existing_config": {
                "enabled": true,
                "api_token": "token-a",
                "endpoint_url": "https://example.com"
            },
            "enabled": false
        });
        let out =
            <Component as QaGuest>::apply_answers(Mode::Upgrade, canonical_cbor_bytes(&answers));
        let out_json: serde_json::Value = decode_cbor(&out).expect("decode apply output");
        assert_eq!(out_json.get("ok"), Some(&serde_json::Value::Bool(true)));
        let config = out_json.get("config").expect("config object");
        assert_eq!(
            config.get("api_token"),
            Some(&serde_json::Value::String("token-a".to_string()))
        );
        assert_eq!(config.get("enabled"), Some(&serde_json::Value::Bool(false)));
    }
}
