use base64::{Engine as _, engine::general_purpose::STANDARD};
use greentic_types::messaging::universal_dto::{
    EncodeInV1, HttpInV1, HttpOutV1, ProviderPayloadV1, RenderPlanInV1, RenderPlanOutV1,
    SendPayloadInV1, SendPayloadResultV1,
};
use greentic_types::{
    Actor, ChannelMessageEnvelope, Destination, EnvId, MessageMetadata, TenantCtx, TenantId,
};
use provider_common::component_v0_6::{
    DescribePayload, I18nText, OperationDescriptor, QaQuestionSpec, QaSpec, RedactionRule,
    SchemaField, SchemaIr, canonical_cbor_bytes, decode_cbor, schema_hash,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

mod bindings {
    wit_bindgen::generate!({
        path: "wit/messaging-provider-slack",
        world: "component-v0-v6-v0",
        generate_all
    });
}

use bindings::greentic::http::http_client as client;
use bindings::greentic::secrets_store::secrets_store;

const PROVIDER_ID: &str = "messaging-provider-slack";
const PROVIDER_TYPE: &str = "messaging.slack.api";
const WORLD_ID: &str = "component-v0-v6-v0";
const DEFAULT_API_BASE: &str = "https://slack.com/api";
const DEFAULT_BOT_TOKEN_KEY: &str = "SLACK_BOT_TOKEN";

const I18N_KEYS: &[&str] = &[
    "slack.op.run.title",
    "slack.op.run.description",
    "slack.op.send.title",
    "slack.op.send.description",
    "slack.op.reply.title",
    "slack.op.reply.description",
    "slack.op.ingest_http.title",
    "slack.op.ingest_http.description",
    "slack.op.render_plan.title",
    "slack.op.render_plan.description",
    "slack.op.encode.title",
    "slack.op.encode.description",
    "slack.op.send_payload.title",
    "slack.op.send_payload.description",
    "slack.schema.input.title",
    "slack.schema.input.description",
    "slack.schema.input.message.title",
    "slack.schema.input.message.description",
    "slack.schema.output.title",
    "slack.schema.output.description",
    "slack.schema.output.ok.title",
    "slack.schema.output.ok.description",
    "slack.schema.output.message_id.title",
    "slack.schema.output.message_id.description",
    "slack.schema.config.title",
    "slack.schema.config.description",
    "slack.schema.config.enabled.title",
    "slack.schema.config.enabled.description",
    "slack.schema.config.default_channel.title",
    "slack.schema.config.default_channel.description",
    "slack.schema.config.public_base_url.title",
    "slack.schema.config.public_base_url.description",
    "slack.schema.config.api_base_url.title",
    "slack.schema.config.api_base_url.description",
    "slack.schema.config.bot_token.title",
    "slack.schema.config.bot_token.description",
    "slack.qa.default.title",
    "slack.qa.setup.title",
    "slack.qa.upgrade.title",
    "slack.qa.remove.title",
    "slack.qa.setup.enabled",
    "slack.qa.setup.public_base_url",
    "slack.qa.setup.api_base_url",
    "slack.qa.setup.bot_token",
    "slack.qa.setup.default_channel",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderConfig {
    #[serde(default = "default_enabled")]
    enabled: bool,
    #[serde(default)]
    default_channel: Option<String>,
    public_base_url: String,
    #[serde(default)]
    api_base_url: Option<String>,
    bot_token: String,
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
struct ProviderConfigOut {
    enabled: bool,
    default_channel: Option<String>,
    public_base_url: String,
    api_base_url: String,
    bot_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RemovePlan {
    remove_all: bool,
    cleanup: Vec<String>,
}

fn default_enabled() -> bool {
    true
}

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
        let output_json = match op.as_str() {
            "run" | "send" => handle_send(&input_json, false),
            "reply" => handle_send(&input_json, true),
            "ingest_http" => ingest_http(&input_json),
            "render_plan" => render_plan(&input_json),
            "encode" => encode_op(&input_json),
            "send_payload" => send_payload(&input_json),
            other => json_bytes(&json!({"ok": false, "error": format!("unsupported op: {other}")})),
        };

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
                        "best_effort_revoke_webhooks".to_string(),
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
        let mut messages = serde_json::Map::new();
        for (key, value) in [
            ("slack.op.run.title", "Run"),
            ("slack.op.run.description", "Run slack provider operation"),
            ("slack.op.send.title", "Send"),
            ("slack.op.send.description", "Send a Slack message"),
            ("slack.op.reply.title", "Reply"),
            ("slack.op.reply.description", "Reply in a Slack thread"),
            ("slack.op.ingest_http.title", "Ingest HTTP"),
            (
                "slack.op.ingest_http.description",
                "Normalize Slack webhook payload",
            ),
            ("slack.op.render_plan.title", "Render Plan"),
            (
                "slack.op.render_plan.description",
                "Render universal message plan",
            ),
            ("slack.op.encode.title", "Encode"),
            (
                "slack.op.encode.description",
                "Encode universal payload for Slack",
            ),
            ("slack.op.send_payload.title", "Send Payload"),
            (
                "slack.op.send_payload.description",
                "Send encoded payload to Slack API",
            ),
            ("slack.schema.input.title", "Slack input"),
            (
                "slack.schema.input.description",
                "Input for Slack run/send operations",
            ),
            ("slack.schema.input.message.title", "Message"),
            ("slack.schema.input.message.description", "Message text"),
            ("slack.schema.output.title", "Slack output"),
            (
                "slack.schema.output.description",
                "Result of Slack operation",
            ),
            ("slack.schema.output.ok.title", "Success"),
            (
                "slack.schema.output.ok.description",
                "Whether operation succeeded",
            ),
            ("slack.schema.output.message_id.title", "Message ID"),
            (
                "slack.schema.output.message_id.description",
                "Slack timestamp identifier",
            ),
            ("slack.schema.config.title", "Slack config"),
            (
                "slack.schema.config.description",
                "Slack provider configuration",
            ),
            ("slack.schema.config.enabled.title", "Enabled"),
            (
                "slack.schema.config.enabled.description",
                "Enable this provider",
            ),
            (
                "slack.schema.config.default_channel.title",
                "Default channel",
            ),
            (
                "slack.schema.config.default_channel.description",
                "Channel used when destination is omitted",
            ),
            (
                "slack.schema.config.public_base_url.title",
                "Public base URL",
            ),
            (
                "slack.schema.config.public_base_url.description",
                "Public URL for callbacks",
            ),
            ("slack.schema.config.api_base_url.title", "API base URL"),
            (
                "slack.schema.config.api_base_url.description",
                "Slack API base URL",
            ),
            ("slack.schema.config.bot_token.title", "Bot token"),
            (
                "slack.schema.config.bot_token.description",
                "Bot token for Slack API calls",
            ),
            ("slack.qa.default.title", "Default"),
            ("slack.qa.setup.title", "Setup"),
            ("slack.qa.upgrade.title", "Upgrade"),
            ("slack.qa.remove.title", "Remove"),
            ("slack.qa.setup.enabled", "Enable provider"),
            ("slack.qa.setup.public_base_url", "Public base URL"),
            ("slack.qa.setup.api_base_url", "API base URL"),
            ("slack.qa.setup.bot_token", "Bot token"),
            ("slack.qa.setup.default_channel", "Default channel"),
        ] {
            messages.insert(key.to_string(), Value::String(value.to_string()));
        }

        canonical_cbor_bytes(&json!({
            "locale": locale,
            "messages": Value::Object(messages),
        }))
    }
}

// Backward-compatible schema-core-api export for operator v0.4.x
impl bindings::exports::greentic::provider_schema_core::schema_core_api::Guest for Component {
    fn describe() -> Vec<u8> {
        serde_json::to_vec(&build_describe_payload()).unwrap_or_default()
    }

    fn validate_config(_config_json: Vec<u8>) -> Vec<u8> {
        json_bytes(&serde_json::json!({"ok": true}))
    }

    fn healthcheck() -> Vec<u8> {
        json_bytes(&serde_json::json!({"status": "healthy"}))
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
            other => json_bytes(
                &serde_json::json!({"ok": false, "error": format!("unsupported op: {other}")}),
            ),
        }
    }
}

bindings::export!(Component with_types_in bindings);

fn build_describe_payload() -> DescribePayload {
    let input_schema = input_schema();
    let output_schema = output_schema();
    let config_schema = config_schema();

    DescribePayload {
        provider: PROVIDER_ID.to_string(),
        world: WORLD_ID.to_string(),
        operations: vec![
            op("run", "slack.op.run.title", "slack.op.run.description"),
            op("send", "slack.op.send.title", "slack.op.send.description"),
            op(
                "reply",
                "slack.op.reply.title",
                "slack.op.reply.description",
            ),
            op(
                "ingest_http",
                "slack.op.ingest_http.title",
                "slack.op.ingest_http.description",
            ),
            op(
                "render_plan",
                "slack.op.render_plan.title",
                "slack.op.render_plan.description",
            ),
            op(
                "encode",
                "slack.op.encode.title",
                "slack.op.encode.description",
            ),
            op(
                "send_payload",
                "slack.op.send_payload.title",
                "slack.op.send_payload.description",
            ),
        ],
        input_schema: input_schema.clone(),
        output_schema: output_schema.clone(),
        config_schema: config_schema.clone(),
        redactions: vec![RedactionRule {
            path: "$.bot_token".to_string(),
            strategy: "replace".to_string(),
        }],
        schema_hash: schema_hash(&input_schema, &output_schema, &config_schema),
    }
}

fn build_qa_spec(mode: bindings::exports::greentic::component::qa::Mode) -> QaSpec {
    use bindings::exports::greentic::component::qa::Mode;

    match mode {
        Mode::Default => QaSpec {
            mode: "default".to_string(),
            title: i18n("slack.qa.default.title"),
            questions: vec![
                QaQuestionSpec {
                    key: "public_base_url".to_string(),
                    text: i18n("slack.qa.setup.public_base_url"),
                    required: true,
                },
                QaQuestionSpec {
                    key: "bot_token".to_string(),
                    text: i18n("slack.qa.setup.bot_token"),
                    required: true,
                },
            ],
        },
        Mode::Setup => QaSpec {
            mode: "setup".to_string(),
            title: i18n("slack.qa.setup.title"),
            questions: vec![
                QaQuestionSpec {
                    key: "enabled".to_string(),
                    text: i18n("slack.qa.setup.enabled"),
                    required: true,
                },
                QaQuestionSpec {
                    key: "public_base_url".to_string(),
                    text: i18n("slack.qa.setup.public_base_url"),
                    required: true,
                },
                QaQuestionSpec {
                    key: "api_base_url".to_string(),
                    text: i18n("slack.qa.setup.api_base_url"),
                    required: true,
                },
                QaQuestionSpec {
                    key: "bot_token".to_string(),
                    text: i18n("slack.qa.setup.bot_token"),
                    required: true,
                },
                QaQuestionSpec {
                    key: "default_channel".to_string(),
                    text: i18n("slack.qa.setup.default_channel"),
                    required: false,
                },
            ],
        },
        Mode::Upgrade => QaSpec {
            mode: "upgrade".to_string(),
            title: i18n("slack.qa.upgrade.title"),
            questions: vec![
                QaQuestionSpec {
                    key: "enabled".to_string(),
                    text: i18n("slack.qa.setup.enabled"),
                    required: false,
                },
                QaQuestionSpec {
                    key: "public_base_url".to_string(),
                    text: i18n("slack.qa.setup.public_base_url"),
                    required: false,
                },
                QaQuestionSpec {
                    key: "api_base_url".to_string(),
                    text: i18n("slack.qa.setup.api_base_url"),
                    required: false,
                },
                QaQuestionSpec {
                    key: "bot_token".to_string(),
                    text: i18n("slack.qa.setup.bot_token"),
                    required: false,
                },
                QaQuestionSpec {
                    key: "default_channel".to_string(),
                    text: i18n("slack.qa.setup.default_channel"),
                    required: false,
                },
            ],
        },
        Mode::Remove => QaSpec {
            mode: "remove".to_string(),
            title: i18n("slack.qa.remove.title"),
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
                title: i18n("slack.schema.input.message.title"),
                description: i18n("slack.schema.input.message.description"),
                format: None,
                secret: false,
            },
        },
    );

    SchemaIr::Object {
        title: i18n("slack.schema.input.title"),
        description: i18n("slack.schema.input.description"),
        fields,
        additional_properties: true,
    }
}

fn output_schema() -> SchemaIr {
    let mut fields = BTreeMap::new();
    fields.insert(
        "ok".to_string(),
        SchemaField {
            required: true,
            schema: SchemaIr::Bool {
                title: i18n("slack.schema.output.ok.title"),
                description: i18n("slack.schema.output.ok.description"),
            },
        },
    );
    fields.insert(
        "message_id".to_string(),
        SchemaField {
            required: false,
            schema: SchemaIr::String {
                title: i18n("slack.schema.output.message_id.title"),
                description: i18n("slack.schema.output.message_id.description"),
                format: None,
                secret: false,
            },
        },
    );

    SchemaIr::Object {
        title: i18n("slack.schema.output.title"),
        description: i18n("slack.schema.output.description"),
        fields,
        additional_properties: true,
    }
}

fn config_schema() -> SchemaIr {
    let mut fields = BTreeMap::new();
    fields.insert(
        "enabled".to_string(),
        SchemaField {
            required: true,
            schema: SchemaIr::Bool {
                title: i18n("slack.schema.config.enabled.title"),
                description: i18n("slack.schema.config.enabled.description"),
            },
        },
    );
    fields.insert(
        "default_channel".to_string(),
        SchemaField {
            required: false,
            schema: SchemaIr::String {
                title: i18n("slack.schema.config.default_channel.title"),
                description: i18n("slack.schema.config.default_channel.description"),
                format: None,
                secret: false,
            },
        },
    );
    fields.insert(
        "public_base_url".to_string(),
        SchemaField {
            required: true,
            schema: SchemaIr::String {
                title: i18n("slack.schema.config.public_base_url.title"),
                description: i18n("slack.schema.config.public_base_url.description"),
                format: Some("uri".to_string()),
                secret: false,
            },
        },
    );
    fields.insert(
        "api_base_url".to_string(),
        SchemaField {
            required: true,
            schema: SchemaIr::String {
                title: i18n("slack.schema.config.api_base_url.title"),
                description: i18n("slack.schema.config.api_base_url.description"),
                format: Some("uri".to_string()),
                secret: false,
            },
        },
    );
    fields.insert(
        "bot_token".to_string(),
        SchemaField {
            required: true,
            schema: SchemaIr::String {
                title: i18n("slack.schema.config.bot_token.title"),
                description: i18n("slack.schema.config.bot_token.description"),
                format: None,
                secret: true,
            },
        },
    );

    SchemaIr::Object {
        title: i18n("slack.schema.config.title"),
        description: i18n("slack.schema.config.description"),
        fields,
        additional_properties: false,
    }
}

fn op(name: &str, title: &str, description: &str) -> OperationDescriptor {
    OperationDescriptor {
        name: name.to_string(),
        title: i18n(title),
        description: i18n(description),
    }
}

fn i18n(key: &str) -> I18nText {
    I18nText {
        key: key.to_string(),
    }
}

fn existing_config_from_answers(answers: &Value) -> Option<ProviderConfigOut> {
    answers
        .get("existing_config")
        .or_else(|| answers.get("config"))
        .and_then(|value| serde_json::from_value(value.clone()).ok())
}

fn optional_string_from(answers: &Value, key: &str) -> Option<String> {
    answers
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn string_or_default(answers: &Value, key: &str, fallback: &str) -> String {
    answers
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

fn default_config_out() -> ProviderConfigOut {
    ProviderConfigOut {
        enabled: true,
        default_channel: None,
        public_base_url: String::new(),
        api_base_url: DEFAULT_API_BASE.to_string(),
        bot_token: String::new(),
    }
}

fn validate_config_out(config: &ProviderConfigOut) -> Result<(), String> {
    if config.public_base_url.trim().is_empty() {
        return Err("invalid config: public_base_url cannot be empty".to_string());
    }
    if !(config.public_base_url.starts_with("http://")
        || config.public_base_url.starts_with("https://"))
    {
        return Err("invalid config: public_base_url must be an absolute URL".to_string());
    }
    if config.bot_token.trim().is_empty() {
        return Err("invalid config: bot_token cannot be empty".to_string());
    }
    if config.api_base_url.trim().is_empty() {
        return Err("invalid config: api_base_url cannot be empty".to_string());
    }
    if !(config.api_base_url.starts_with("http://") || config.api_base_url.starts_with("https://"))
    {
        return Err("invalid config: api_base_url must be an absolute URL".to_string());
    }
    Ok(())
}

fn handle_send(input_json: &[u8], is_reply: bool) -> Vec<u8> {
    let parsed: Value = match serde_json::from_slice(input_json) {
        Ok(val) => val,
        Err(err) => {
            return json_bytes(&json!({"ok": false, "error": format!("invalid json: {err}")}));
        }
    };

    let cfg = match load_config(&parsed) {
        Ok(cfg) => cfg,
        Err(err) => return json_bytes(&json!({"ok": false, "error": err})),
    };
    if !cfg.enabled {
        return json_bytes(&json!({"ok": false, "error": "provider disabled by config"}));
    }

    let envelope: ChannelMessageEnvelope = match serde_json::from_slice(input_json) {
        Ok(env) => env,
        Err(_) => match build_synthetic_envelope(&parsed, &cfg) {
            Ok(env) => env,
            Err(message) => return json_bytes(&json!({"ok": false, "error": message})),
        },
    };

    if !envelope.attachments.is_empty() {
        return json_bytes(&json!({"ok": false, "error": "attachments not supported"}));
    }

    let text = envelope
        .text
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let text = match text {
        Some(value) => value,
        None => return json_bytes(&json!({"ok": false, "error": "text required"})),
    };

    let destination = envelope.to.first().cloned().or_else(|| {
        cfg.default_channel.clone().map(|channel| Destination {
            id: channel,
            kind: Some("channel".into()),
        })
    });
    let destination = match destination {
        Some(dest) => dest,
        None => return json_bytes(&json!({"ok": false, "error": "destination required"})),
    };

    let dest_id = destination.id.trim();
    if dest_id.is_empty() {
        return json_bytes(&json!({"ok": false, "error": "destination id required"}));
    }
    let dest_id = dest_id.to_string();
    let kind = destination.kind.as_deref().unwrap_or("channel");
    if kind != "channel" && kind != "user" && !kind.is_empty() {
        return json_bytes(&json!({
            "ok": false,
            "error": format!("unsupported destination kind: {kind}")
        }));
    }

    let thread_ts = if is_reply {
        parsed
            .get("thread_id")
            .or_else(|| parsed.get("reply_to_id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    } else {
        None
    };

    let (format, blocks) = parse_blocks(&parsed);

    let token = resolve_bot_token(&cfg);
    let api_base = cfg
        .api_base_url
        .clone()
        .unwrap_or_else(|| DEFAULT_API_BASE.to_string());
    let url = format!("{}/chat.postMessage", api_base);
    let mut payload = json!({
        "channel": dest_id,
        "text": text,
    });
    if let Some(ts) = thread_ts {
        payload
            .as_object_mut()
            .expect("payload object")
            .insert("thread_ts".into(), Value::String(ts));
    }
    if format.as_deref() == Some("slack_blocks")
        && let Some(b) = blocks
    {
        payload
            .as_object_mut()
            .expect("payload object")
            .insert("blocks".into(), b);
    }

    let request = client::Request {
        method: "POST".into(),
        url,
        headers: vec![
            ("Content-Type".into(), "application/json".into()),
            ("Authorization".into(), format!("Bearer {token}")),
        ],
        body: Some(serde_json::to_vec(&payload).unwrap_or_else(|_| b"{}".to_vec())),
    };

    let resp = match client::send(&request, None, None) {
        Ok(resp) => resp,
        Err(err) => {
            return json_bytes(
                &json!({"ok": false, "error": format!("transport error: {}", err.message)}),
            );
        }
    };

    if resp.status < 200 || resp.status >= 300 {
        return json_bytes(
            &json!({"ok": false, "error": format!("slack returned status {}", resp.status)}),
        );
    }

    let body = resp.body.unwrap_or_default();
    let body_json: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let ts = body_json
        .get("ts")
        .or_else(|| body_json.get("message").and_then(|m| m.get("ts")))
        .and_then(|v| v.as_str())
        .unwrap_or("pending-ts")
        .to_string();
    let provider_message_id = format!("slack:{ts}");

    let result = json!({
        "ok": true,
        "status": if is_reply {"replied"} else {"sent"},
        "provider_type": PROVIDER_TYPE,
        "public_base_url": cfg.public_base_url,
        "message_id": ts,
        "provider_message_id": provider_message_id,
        "response": body_json
    });
    json_bytes(&result)
}

fn resolve_bot_token(cfg: &ProviderConfig) -> String {
    if !cfg.bot_token.trim().is_empty() {
        return cfg.bot_token.clone();
    }
    get_secret_string(DEFAULT_BOT_TOKEN_KEY).unwrap_or_default()
}

fn parse_blocks(parsed: &Value) -> (Option<String>, Option<Value>) {
    let format = parsed
        .get("rich")
        .and_then(|v| v.get("format"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let blocks = parsed.get("rich").and_then(|v| v.get("blocks")).cloned();
    (format, blocks)
}

#[cfg(test)]
fn parse_config_bytes(bytes: &[u8]) -> Result<ProviderConfig, String> {
    serde_json::from_slice::<ProviderConfig>(bytes).map_err(|e| format!("invalid config: {e}"))
}

fn parse_config_value(val: &Value) -> Result<ProviderConfig, String> {
    serde_json::from_value::<ProviderConfig>(val.clone())
        .map_err(|e| format!("invalid config: {e}"))
}

fn load_config(input: &Value) -> Result<ProviderConfig, String> {
    if let Some(cfg) = input.get("config") {
        return parse_config_value(cfg);
    }

    let mut partial = serde_json::Map::new();
    for key in [
        "enabled",
        "default_channel",
        "public_base_url",
        "api_base_url",
        "bot_token",
    ] {
        if let Some(v) = input.get(key) {
            partial.insert(key.to_string(), v.clone());
        }
    }
    if !partial.is_empty() {
        return parse_config_value(&Value::Object(partial));
    }

    Err("missing config: expected `config` or top-level config fields".to_string())
}

fn build_synthetic_envelope(
    parsed: &Value,
    cfg: &ProviderConfig,
) -> Result<ChannelMessageEnvelope, String> {
    let destination = parse_destination(parsed).or_else(|| {
        cfg.default_channel.clone().map(|channel| Destination {
            id: channel,
            kind: Some("channel".to_string()),
        })
    });
    let destination = destination.ok_or_else(|| "channel required".to_string())?;

    let env = EnvId::try_from("manual").expect("manual env id");
    let tenant = TenantId::try_from("manual").expect("manual tenant id");
    let mut metadata = MessageMetadata::new();
    metadata.insert("channel".to_string(), destination.id.clone());
    if let Some(kind) = &destination.kind {
        metadata.insert("destination_kind".to_string(), kind.clone());
    }

    let text = parsed
        .get("text")
        .and_then(|value| value.as_str())
        .map(|s| s.to_string());

    Ok(ChannelMessageEnvelope {
        id: "synthetic-slack-envelope".to_string(),
        tenant: TenantCtx::new(env, tenant),
        channel: destination.id.clone(),
        session_id: destination.id.clone(),
        reply_scope: None,
        from: None,
        to: vec![destination],
        correlation_id: None,
        text,
        attachments: Vec::new(),
        metadata,
    })
}

fn parse_destination(parsed: &Value) -> Option<Destination> {
    let to_value = parsed.get("to")?;
    if let Some(id) = to_value.as_str() {
        let trimmed = id.trim();
        if trimmed.is_empty() {
            return None;
        }
        return Some(Destination {
            id: trimmed.to_string(),
            kind: Some("channel".to_string()),
        });
    }

    let obj = to_value.as_object()?;
    let id = obj
        .get("id")
        .and_then(|value| value.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let kind = obj
        .get("kind")
        .and_then(|value| value.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    id.map(|id| Destination {
        id,
        kind: kind.or_else(|| Some("channel".to_string())),
    })
}

fn json_bytes<T: serde::Serialize>(value: &T) -> Vec<u8> {
    serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec())
}

fn ingest_http(input_json: &[u8]) -> Vec<u8> {
    let request = match serde_json::from_slice::<HttpInV1>(input_json) {
        Ok(req) => req,
        Err(err) => return http_out_error(400, &format!("invalid http input: {err}")),
    };
    let body_bytes = match STANDARD.decode(&request.body_b64) {
        Ok(bytes) => bytes,
        Err(err) => return http_out_error(400, &format!("invalid body encoding: {err}")),
    };
    let body_val: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    let payload = body_val.get("body").cloned().unwrap_or(Value::Null);
    let text = payload
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let channel = payload
        .get("channel")
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    let sender = payload
        .get("user")
        .or_else(|| payload.get("user_id"))
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    let envelope = build_slack_envelope(text, channel.clone(), sender);
    let normalized = json!({
        "ok": true,
        "event": body_val,
        "channel": channel,
    });
    let normalized_bytes = serde_json::to_vec(&normalized).unwrap_or_else(|_| b"{}".to_vec());
    let out = HttpOutV1 {
        status: 200,
        headers: Vec::new(),
        body_b64: STANDARD.encode(&normalized_bytes),
        events: vec![envelope],
    };
    json_bytes(&out)
}

fn render_plan(input_json: &[u8]) -> Vec<u8> {
    let plan_in = match serde_json::from_slice::<RenderPlanInV1>(input_json) {
        Ok(value) => value,
        Err(err) => return render_plan_error(&format!("invalid render input: {err}")),
    };
    let ac_summary = provider_common::extract_ac_text_summary(&plan_in.message.metadata);
    let summary = ac_summary
        .or_else(|| {
            plan_in
                .message
                .text
                .clone()
                .filter(|t| !t.trim().is_empty())
        })
        .unwrap_or_else(|| "slack message".to_string());
    let mut warnings: Vec<Value> = Vec::new();
    if plan_in.message.metadata.contains_key("adaptive_card") {
        warnings
            .push(json!({"code": "adaptive_cards_not_supported", "message": null, "path": null}));
    }
    let plan_obj = json!({
        "tier": "TierD",
        "summary_text": summary,
        "actions": [],
        "attachments": [],
        "warnings": warnings,
        "debug": plan_in.metadata,
    });
    let plan_json =
        serde_json::to_string(&plan_obj).unwrap_or_else(|_| "{\"tier\":\"TierD\"}".to_string());
    let plan_out = RenderPlanOutV1 { plan_json };
    json_bytes(&json!({"ok": true, "plan": plan_out}))
}

fn encode_op(input_json: &[u8]) -> Vec<u8> {
    let encode_in = match serde_json::from_slice::<EncodeInV1>(input_json) {
        Ok(value) => value,
        Err(err) => return encode_error(&format!("invalid encode input: {err}")),
    };
    let channel = encode_in
        .message
        .to
        .first()
        .map(|d| d.id.clone())
        .unwrap_or_default();
    if channel.is_empty() {
        return encode_error("destination (to) required");
    }
    let text = encode_in
        .message
        .text
        .clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| "slack universal payload".to_string());
    let url = format!("{}/chat.postMessage", DEFAULT_API_BASE);
    let body = json!({
        "channel": channel,
        "text": text,
    });
    let body_bytes = serde_json::to_vec(&body).unwrap_or_else(|_| b"{}".to_vec());
    let mut metadata = BTreeMap::new();
    metadata.insert("url".to_string(), Value::String(url));
    metadata.insert("method".to_string(), Value::String("POST".to_string()));
    metadata.insert("channel".to_string(), Value::String(channel));
    let payload = ProviderPayloadV1 {
        content_type: "application/json".to_string(),
        body_b64: STANDARD.encode(&body_bytes),
        metadata,
    };
    json_bytes(&json!({"ok": true, "payload": payload}))
}

fn send_payload(input_json: &[u8]) -> Vec<u8> {
    let send_in = match serde_json::from_slice::<SendPayloadInV1>(input_json) {
        Ok(value) => value,
        Err(err) => {
            return send_payload_error(&format!("invalid send_payload input: {err}"), false);
        }
    };
    if send_in.provider_type != PROVIDER_TYPE {
        return send_payload_error("provider type mismatch", false);
    }
    let ProviderPayloadV1 {
        content_type,
        body_b64,
        metadata,
    } = send_in.payload;
    let url = metadata_string(&metadata, "url")
        .unwrap_or_else(|| format!("{}/chat.postMessage", DEFAULT_API_BASE));
    let method = metadata_string(&metadata, "method").unwrap_or_else(|| "POST".to_string());
    let body_bytes = match STANDARD.decode(&body_b64) {
        Ok(bytes) => bytes,
        Err(err) => return send_payload_error(&format!("payload decode failed: {err}"), false),
    };
    let token = match get_secret_string(DEFAULT_BOT_TOKEN_KEY) {
        Ok(value) => value,
        Err(err) => return send_payload_error(&err, false),
    };
    let request = client::Request {
        method,
        url,
        headers: vec![
            ("Content-Type".into(), content_type.clone()),
            ("Authorization".into(), format!("Bearer {token}")),
        ],
        body: Some(body_bytes),
    };
    let resp = match client::send(&request, None, None) {
        Ok(value) => value,
        Err(err) => {
            return send_payload_error(&format!("transport error: {}", err.message), true);
        }
    };
    if resp.status < 200 || resp.status >= 300 {
        return send_payload_error(
            &format!("slack returned status {}", resp.status),
            resp.status >= 500,
        );
    }
    send_payload_success()
}

fn metadata_string(metadata: &BTreeMap<String, Value>, key: &str) -> Option<String> {
    metadata
        .get(key)
        .and_then(|value| value.as_str().map(|s| s.to_string()))
}

fn http_out_error(status: u16, message: &str) -> Vec<u8> {
    let out = HttpOutV1 {
        status,
        headers: Vec::new(),
        body_b64: STANDARD.encode(message.as_bytes()),
        events: Vec::new(),
    };
    json_bytes(&out)
}

fn render_plan_error(message: &str) -> Vec<u8> {
    json_bytes(&json!({"ok": false, "error": message}))
}

fn encode_error(message: &str) -> Vec<u8> {
    json_bytes(&json!({"ok": false, "error": message}))
}

fn build_slack_envelope(
    text: String,
    channel: Option<String>,
    sender: Option<String>,
) -> ChannelMessageEnvelope {
    let env = EnvId::try_from("default").expect("env id");
    let tenant = TenantId::try_from("default").expect("tenant id");
    let mut metadata = MessageMetadata::new();
    metadata.insert("universal".to_string(), "true".to_string());
    if let Some(channel_id) = &channel {
        metadata.insert("channel".to_string(), channel_id.clone());
    }
    if let Some(sender_id) = &sender {
        metadata.insert("from".to_string(), sender_id.clone());
    }
    let channel_name = channel.clone().unwrap_or_else(|| "slack".to_string());
    let actor = sender.map(|id| Actor {
        id,
        kind: Some("user".into()),
    });
    ChannelMessageEnvelope {
        id: format!("slack-{channel_name}"),
        tenant: TenantCtx::new(env.clone(), tenant.clone()),
        channel: channel_name.clone(),
        session_id: channel_name,
        reply_scope: None,
        from: actor,
        to: Vec::new(),
        correlation_id: None,
        text: Some(text),
        attachments: Vec::new(),
        metadata,
    }
}

fn send_payload_error(message: &str, retryable: bool) -> Vec<u8> {
    let result = SendPayloadResultV1 {
        ok: false,
        message: Some(message.to_string()),
        retryable,
    };
    json_bytes(&result)
}

fn send_payload_success() -> Vec<u8> {
    let result = SendPayloadResultV1 {
        ok: true,
        message: None,
        retryable: false,
    };
    json_bytes(&result)
}

fn get_secret_string(key: &str) -> Result<String, String> {
    match secrets_store::get(key) {
        Ok(Some(bytes)) => String::from_utf8(bytes).map_err(|_| "secret not valid utf-8".into()),
        Ok(None) => Err(format!("missing secret: {key}")),
        Err(e) => Err(format!("secret store error: {e:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn parse_config_rejects_unknown() {
        let cfg = br#"{"enabled":true,"public_base_url":"https://x","api_base_url":"https://slack.com/api","bot_token":"x","unknown":true}"#;
        let err = parse_config_bytes(cfg).unwrap_err();
        assert!(err.contains("unknown field"));
    }

    #[test]
    fn schema_hash_is_stable() {
        let describe = build_describe_payload();
        assert_eq!(
            describe.schema_hash,
            "0d7cbda46632fd39f7ade4774c1dee9a7deebd7b382b5c785a384b1899faa519"
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
        assert_eq!(keys, vec!["public_base_url", "bot_token"]);
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
            "bot_token": "token-a"
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
