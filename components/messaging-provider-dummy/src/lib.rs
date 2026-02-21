use provider_common::component_v0_6::{
    DescribePayload, I18nText, OperationDescriptor, QaQuestionSpec, QaSpec, RedactionRule,
    SchemaField, SchemaIr, canonical_cbor_bytes, decode_cbor, schema_hash,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
                return canonical_cbor_bytes(&ApplyAnswersResult {
                    ok: false,
                    config: None,
                    remove: None,
                    diagnostics: Vec::new(),
                    error: Some(format!("invalid answers cbor: {err}")),
                });
            }
        };

        if mode == bindings::exports::greentic::component::qa::Mode::Remove {
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
}

impl bindings::exports::greentic::component::component_i18n::Guest for Component {
    fn i18n_keys() -> Vec<String> {
        I18N_KEYS.iter().map(|key| (*key).to_string()).collect()
    }

    fn i18n_bundle(locale: String) -> Vec<u8> {
        let locale = if locale.trim().is_empty() {
            "en".to_string()
        } else {
            locale
        };
        canonical_cbor_bytes(&serde_json::json!({
            "locale": locale,
            "messages": {
                "dummy.op.run.title": "Run",
                "dummy.op.run.description": "Run dummy provider operation",
                "dummy.schema.input.title": "Dummy input",
                "dummy.schema.input.description": "Input for dummy provider run",
                "dummy.schema.input.message.title": "Message",
                "dummy.schema.input.message.description": "Message text to hash",
                "dummy.schema.output.title": "Dummy output",
                "dummy.schema.output.description": "Result of dummy provider run",
                "dummy.schema.output.ok.title": "Success",
                "dummy.schema.output.ok.description": "Whether provider run succeeded",
                "dummy.schema.output.message_id.title": "Message ID",
                "dummy.schema.output.message_id.description": "Deterministic message identifier",
                "dummy.schema.config.title": "Dummy config",
                "dummy.schema.config.description": "Dummy provider configuration",
                "dummy.schema.config.enabled.title": "Enabled",
                "dummy.schema.config.enabled.description": "Enable this provider",
                "dummy.schema.config.api_token.title": "API token",
                "dummy.schema.config.api_token.description": "Tenant token for dummy provider",
                "dummy.schema.config.endpoint_url.title": "Endpoint URL",
                "dummy.schema.config.endpoint_url.description": "Dummy endpoint URL",
                "dummy.qa.default.title": "Default",
                "dummy.qa.setup.title": "Setup",
                "dummy.qa.upgrade.title": "Upgrade",
                "dummy.qa.remove.title": "Remove",
                "dummy.qa.setup.enabled": "Enable provider",
                "dummy.qa.setup.api_token": "API token",
                "dummy.qa.setup.endpoint_url": "Endpoint URL"
            }
        }))
    }
}

// Backward-compatible schema-core-api export for operator v0.4.x
impl bindings::exports::greentic::provider_schema_core::schema_core_api::Guest for Component {
    fn describe() -> Vec<u8> {
        serde_json::to_vec(&build_describe_payload()).unwrap_or_default()
    }

    fn validate_config(_config_json: Vec<u8>) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"ok": true})).unwrap_or_default()
    }

    fn healthcheck() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"status": "healthy"})).unwrap_or_default()
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ApplyAnswersResult {
    ok: bool,
    config: Option<ProviderConfig>,
    remove: Option<RemovePlan>,
    diagnostics: Vec<String>,
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RemovePlan {
    remove_all: bool,
    cleanup: Vec<String>,
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

fn build_qa_spec(mode: bindings::exports::greentic::component::qa::Mode) -> QaSpec {
    use bindings::exports::greentic::component::qa::Mode;

    match mode {
        Mode::Default => QaSpec {
            mode: "default".to_string(),
            title: i18n("dummy.qa.default.title"),
            questions: vec![
                QaQuestionSpec {
                    key: "api_token".to_string(),
                    text: i18n("dummy.qa.setup.api_token"),
                    required: true,
                },
                QaQuestionSpec {
                    key: "endpoint_url".to_string(),
                    text: i18n("dummy.qa.setup.endpoint_url"),
                    required: true,
                },
            ],
        },
        Mode::Setup => QaSpec {
            mode: "setup".to_string(),
            title: i18n("dummy.qa.setup.title"),
            questions: vec![
                QaQuestionSpec {
                    key: "enabled".to_string(),
                    text: i18n("dummy.qa.setup.enabled"),
                    required: true,
                },
                QaQuestionSpec {
                    key: "api_token".to_string(),
                    text: i18n("dummy.qa.setup.api_token"),
                    required: true,
                },
                QaQuestionSpec {
                    key: "endpoint_url".to_string(),
                    text: i18n("dummy.qa.setup.endpoint_url"),
                    required: true,
                },
            ],
        },
        Mode::Upgrade => QaSpec {
            mode: "upgrade".to_string(),
            title: i18n("dummy.qa.upgrade.title"),
            questions: vec![
                QaQuestionSpec {
                    key: "enabled".to_string(),
                    text: i18n("dummy.qa.setup.enabled"),
                    required: false,
                },
                QaQuestionSpec {
                    key: "api_token".to_string(),
                    text: i18n("dummy.qa.setup.api_token"),
                    required: false,
                },
                QaQuestionSpec {
                    key: "endpoint_url".to_string(),
                    text: i18n("dummy.qa.setup.endpoint_url"),
                    required: false,
                },
            ],
        },
        Mode::Remove => QaSpec {
            mode: "remove".to_string(),
            title: i18n("dummy.qa.remove.title"),
            questions: Vec::new(),
        },
    }
}

fn input_schema() -> SchemaIr {
    let mut fields = BTreeMap::new();
    fields.insert(
        "message".to_string(),
        SchemaField {
            required: true,
            schema: SchemaIr::String {
                title: i18n("dummy.schema.input.message.title"),
                description: i18n("dummy.schema.input.message.description"),
                format: None,
                secret: false,
            },
        },
    );

    SchemaIr::Object {
        title: i18n("dummy.schema.input.title"),
        description: i18n("dummy.schema.input.description"),
        fields,
        additional_properties: false,
    }
}

fn output_schema() -> SchemaIr {
    let mut fields = BTreeMap::new();
    fields.insert(
        "ok".to_string(),
        SchemaField {
            required: true,
            schema: SchemaIr::Bool {
                title: i18n("dummy.schema.output.ok.title"),
                description: i18n("dummy.schema.output.ok.description"),
            },
        },
    );
    fields.insert(
        "message_id".to_string(),
        SchemaField {
            required: false,
            schema: SchemaIr::String {
                title: i18n("dummy.schema.output.message_id.title"),
                description: i18n("dummy.schema.output.message_id.description"),
                format: None,
                secret: false,
            },
        },
    );

    SchemaIr::Object {
        title: i18n("dummy.schema.output.title"),
        description: i18n("dummy.schema.output.description"),
        fields,
        additional_properties: false,
    }
}

fn config_schema() -> SchemaIr {
    let mut fields = BTreeMap::new();
    fields.insert(
        "enabled".to_string(),
        SchemaField {
            required: true,
            schema: SchemaIr::Bool {
                title: i18n("dummy.schema.config.enabled.title"),
                description: i18n("dummy.schema.config.enabled.description"),
            },
        },
    );
    fields.insert(
        "api_token".to_string(),
        SchemaField {
            required: true,
            schema: SchemaIr::String {
                title: i18n("dummy.schema.config.api_token.title"),
                description: i18n("dummy.schema.config.api_token.description"),
                format: None,
                secret: true,
            },
        },
    );
    fields.insert(
        "endpoint_url".to_string(),
        SchemaField {
            required: true,
            schema: SchemaIr::String {
                title: i18n("dummy.schema.config.endpoint_url.title"),
                description: i18n("dummy.schema.config.endpoint_url.description"),
                format: Some("uri".to_string()),
                secret: false,
            },
        },
    );

    SchemaIr::Object {
        title: i18n("dummy.schema.config.title"),
        description: i18n("dummy.schema.config.description"),
        fields,
        additional_properties: false,
    }
}

fn i18n(key: &str) -> I18nText {
    I18nText {
        key: key.to_string(),
    }
}

fn existing_config_from_answers(answers: &serde_json::Value) -> Option<ProviderConfig> {
    answers
        .get("existing_config")
        .cloned()
        .or_else(|| answers.get("config").cloned())
        .and_then(|value| serde_json::from_value::<ProviderConfig>(value).ok())
}

fn string_or_default(answers: &serde_json::Value, key: &str, default: &str) -> String {
    answers
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| default.to_string())
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
    use std::collections::BTreeSet;

    #[test]
    fn schema_hash_is_stable() {
        let describe = build_describe_payload();
        assert_eq!(
            describe.schema_hash,
            "4092e7c953666fbd6a2c172edc5a333e5d0a00f4f4bb173bc052ea270ef70dd5"
        );
    }

    #[test]
    fn describe_passes_strict_rules() {
        let describe = build_describe_payload();
        assert!(!describe.operations.is_empty());

        let expected_hash = schema_hash(
            &describe.input_schema,
            &describe.output_schema,
            &describe.config_schema,
        );
        assert_eq!(describe.schema_hash, expected_hash);

        assert_eq!(describe.world, WORLD_ID);
        assert_eq!(describe.provider, PROVIDER_ID);
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
                assert!(keyset.contains(&question.text.key));
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
            .map(|question| question.key)
            .collect::<Vec<_>>();
        assert_eq!(keys, vec!["api_token", "endpoint_url"]);
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

    #[test]
    fn apply_answers_remove_returns_cleanup_plan() {
        use bindings::exports::greentic::component::qa::Guest as QaGuest;
        use bindings::exports::greentic::component::qa::Mode;
        let out = <Component as QaGuest>::apply_answers(
            Mode::Remove,
            canonical_cbor_bytes(&serde_json::json!({})),
        );
        let out_json: serde_json::Value = decode_cbor(&out).expect("decode apply output");
        assert_eq!(out_json.get("ok"), Some(&serde_json::Value::Bool(true)));
        assert_eq!(out_json.get("config"), Some(&serde_json::Value::Null));
        let cleanup = out_json
            .get("remove")
            .and_then(|value| value.get("cleanup"))
            .and_then(serde_json::Value::as_array)
            .expect("cleanup steps");
        assert!(!cleanup.is_empty());
    }

    #[test]
    fn apply_answers_validates_endpoint_url() {
        use bindings::exports::greentic::component::qa::Guest as QaGuest;
        use bindings::exports::greentic::component::qa::Mode;
        let answers = serde_json::json!({
            "api_token": "token-a",
            "endpoint_url": "not-a-url"
        });
        let out =
            <Component as QaGuest>::apply_answers(Mode::Default, canonical_cbor_bytes(&answers));
        let out_json: serde_json::Value = decode_cbor(&out).expect("decode apply output");
        assert_eq!(out_json.get("ok"), Some(&serde_json::Value::Bool(false)));
        let error = out_json
            .get("error")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        assert!(error.contains("endpoint_url"));
    }
}
