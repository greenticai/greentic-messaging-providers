use base64::{Engine, engine::general_purpose::STANDARD};
use greentic_types::messaging::universal_dto::{
    EncodeInV1, HttpInV1, HttpOutV1, ProviderPayloadV1, RenderPlanInV1, RenderPlanOutV1,
    SendPayloadInV1, SendPayloadResultV1,
};
use greentic_types::{
    Actor, ChannelMessageEnvelope, Destination, EnvId, MessageMetadata, TenantCtx, TenantId,
};
use provider_common::component_v0_6::{
    DescribePayload, I18nText, OperationDescriptor, QaQuestionSpec, QaSpec, RedactionRule,
    SchemaField, SchemaIr, canonical_cbor_bytes, decode_cbor, default_en_i18n_messages,
    schema_hash,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

mod bindings {
    wit_bindgen::generate!({
        path: "wit/messaging-provider-telegram",
        world: "component-v0-v6-v0",
        generate_all
    });
}

use bindings::greentic::http::http_client as client;
use bindings::greentic::secrets_store::secrets_store;

const PROVIDER_ID: &str = "messaging-provider-telegram";
const PROVIDER_TYPE: &str = "messaging.telegram.bot";
const WORLD_ID: &str = "component-v0-v6-v0";
const DEFAULT_API_BASE: &str = "https://api.telegram.org";
const TOKEN_SECRET: &str = "TELEGRAM_BOT_TOKEN";
const I18N_KEYS: &[&str] = &[
    "telegram.op.run.title",
    "telegram.op.run.description",
    "telegram.op.send.title",
    "telegram.op.send.description",
    "telegram.op.reply.title",
    "telegram.op.reply.description",
    "telegram.op.ingest_http.title",
    "telegram.op.ingest_http.description",
    "telegram.op.render_plan.title",
    "telegram.op.render_plan.description",
    "telegram.op.encode.title",
    "telegram.op.encode.description",
    "telegram.op.send_payload.title",
    "telegram.op.send_payload.description",
    "telegram.schema.input.title",
    "telegram.schema.input.description",
    "telegram.schema.input.message.title",
    "telegram.schema.input.message.description",
    "telegram.schema.output.title",
    "telegram.schema.output.description",
    "telegram.schema.output.ok.title",
    "telegram.schema.output.ok.description",
    "telegram.schema.output.message_id.title",
    "telegram.schema.output.message_id.description",
    "telegram.schema.config.title",
    "telegram.schema.config.description",
    "telegram.schema.config.enabled.title",
    "telegram.schema.config.enabled.description",
    "telegram.schema.config.public_base_url.title",
    "telegram.schema.config.public_base_url.description",
    "telegram.schema.config.default_chat_id.title",
    "telegram.schema.config.default_chat_id.description",
    "telegram.schema.config.api_base_url.title",
    "telegram.schema.config.api_base_url.description",
    "telegram.schema.config.bot_token.title",
    "telegram.schema.config.bot_token.description",
    "telegram.qa.default.title",
    "telegram.qa.setup.title",
    "telegram.qa.upgrade.title",
    "telegram.qa.remove.title",
    "telegram.qa.setup.enabled",
    "telegram.qa.setup.public_base_url",
    "telegram.qa.setup.default_chat_id",
    "telegram.qa.setup.api_base_url",
    "telegram.qa.setup.bot_token",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderConfig {
    #[serde(default = "default_enabled")]
    enabled: bool,
    public_base_url: String,
    #[serde(default)]
    default_chat_id: Option<String>,
    #[serde(default)]
    api_base_url: Option<String>,
    #[serde(default)]
    bot_token: Option<String>,
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
    public_base_url: String,
    default_chat_id: Option<String>,
    api_base_url: String,
    bot_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RemovePlan {
    remove_all: bool,
    cleanup: Vec<String>,
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
            merged.public_base_url =
                string_or_default(&answers, "public_base_url", &merged.public_base_url);
            merged.default_chat_id = optional_string_from(&answers, "default_chat_id")
                .or(merged.default_chat_id.clone());
            merged.api_base_url = string_or_default(&answers, "api_base_url", &merged.api_base_url);
            if merged.api_base_url.trim().is_empty() {
                merged.api_base_url = DEFAULT_API_BASE.to_string();
            }
            merged.bot_token =
                optional_string_from(&answers, "bot_token").or(merged.bot_token.clone());
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
            if has("default_chat_id") {
                merged.default_chat_id = optional_string_from(&answers, "default_chat_id");
            }
            if has("api_base_url") {
                merged.api_base_url =
                    string_or_default(&answers, "api_base_url", &merged.api_base_url);
            }
            if has("bot_token") {
                merged.bot_token = optional_string_from(&answers, "bot_token");
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
        I18N_KEYS.iter().map(|k| (*k).to_string()).collect()
    }

    fn i18n_bundle(locale: String) -> Vec<u8> {
        let locale = if locale.trim().is_empty() {
            "en".to_string()
        } else {
            locale
        };
        let messages = default_en_i18n_messages(I18N_KEYS);
        canonical_cbor_bytes(&json!({"locale": locale, "messages": Value::Object(messages)}))
    }
}

// Backward-compatible schema-core-api export for operator v0.4.x
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

fn default_enabled() -> bool {
    true
}

fn dispatch_json_invoke(op: &str, input_json: &[u8]) -> Vec<u8> {
    match op {
        "send" => handle_send(input_json),
        "reply" => handle_reply(input_json),
        "ingest_http" => ingest_http(input_json),
        "render_plan" => render_plan(input_json),
        "encode" => encode_op(input_json),
        "send_payload" => send_payload(input_json),
        other => json_bytes(&json!({"ok": false, "error": format!("unsupported op: {other}")})),
    }
}

fn build_describe_payload() -> DescribePayload {
    let input_schema = input_schema();
    let output_schema = output_schema();
    let config_schema = config_schema();
    DescribePayload {
        provider: PROVIDER_ID.to_string(),
        world: WORLD_ID.to_string(),
        operations: vec![
            op(
                "run",
                "telegram.op.run.title",
                "telegram.op.run.description",
            ),
            op(
                "send",
                "telegram.op.send.title",
                "telegram.op.send.description",
            ),
            op(
                "reply",
                "telegram.op.reply.title",
                "telegram.op.reply.description",
            ),
            op(
                "ingest_http",
                "telegram.op.ingest_http.title",
                "telegram.op.ingest_http.description",
            ),
            op(
                "render_plan",
                "telegram.op.render_plan.title",
                "telegram.op.render_plan.description",
            ),
            op(
                "encode",
                "telegram.op.encode.title",
                "telegram.op.encode.description",
            ),
            op(
                "send_payload",
                "telegram.op.send_payload.title",
                "telegram.op.send_payload.description",
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
            title: i18n("telegram.qa.default.title"),
            questions: vec![qa_q(
                "public_base_url",
                "telegram.qa.setup.public_base_url",
                true,
            )],
        },
        Mode::Setup => QaSpec {
            mode: "setup".to_string(),
            title: i18n("telegram.qa.setup.title"),
            questions: vec![
                qa_q("enabled", "telegram.qa.setup.enabled", true),
                qa_q("public_base_url", "telegram.qa.setup.public_base_url", true),
                qa_q(
                    "default_chat_id",
                    "telegram.qa.setup.default_chat_id",
                    false,
                ),
                qa_q("api_base_url", "telegram.qa.setup.api_base_url", true),
                qa_q("bot_token", "telegram.qa.setup.bot_token", false),
            ],
        },
        Mode::Upgrade => QaSpec {
            mode: "upgrade".to_string(),
            title: i18n("telegram.qa.upgrade.title"),
            questions: vec![
                qa_q("enabled", "telegram.qa.setup.enabled", false),
                qa_q(
                    "public_base_url",
                    "telegram.qa.setup.public_base_url",
                    false,
                ),
                qa_q(
                    "default_chat_id",
                    "telegram.qa.setup.default_chat_id",
                    false,
                ),
                qa_q("api_base_url", "telegram.qa.setup.api_base_url", false),
                qa_q("bot_token", "telegram.qa.setup.bot_token", false),
            ],
        },
        Mode::Remove => QaSpec {
            mode: "remove".to_string(),
            title: i18n("telegram.qa.remove.title"),
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
                title: i18n("telegram.schema.input.message.title"),
                description: i18n("telegram.schema.input.message.description"),
                format: None,
                secret: false,
            },
        },
    );
    SchemaIr::Object {
        title: i18n("telegram.schema.input.title"),
        description: i18n("telegram.schema.input.description"),
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
                title: i18n("telegram.schema.output.ok.title"),
                description: i18n("telegram.schema.output.ok.description"),
            },
        },
    );
    fields.insert(
        "message_id".to_string(),
        SchemaField {
            required: false,
            schema: SchemaIr::String {
                title: i18n("telegram.schema.output.message_id.title"),
                description: i18n("telegram.schema.output.message_id.description"),
                format: None,
                secret: false,
            },
        },
    );
    SchemaIr::Object {
        title: i18n("telegram.schema.output.title"),
        description: i18n("telegram.schema.output.description"),
        fields,
        additional_properties: true,
    }
}

fn config_schema() -> SchemaIr {
    let mut fields = BTreeMap::new();
    let mut insert = |k: &str, required: bool, schema: SchemaIr| {
        fields.insert(k.to_string(), SchemaField { required, schema });
    };
    insert(
        "enabled",
        true,
        SchemaIr::Bool {
            title: i18n("telegram.schema.config.enabled.title"),
            description: i18n("telegram.schema.config.enabled.description"),
        },
    );
    insert(
        "public_base_url",
        true,
        SchemaIr::String {
            title: i18n("telegram.schema.config.public_base_url.title"),
            description: i18n("telegram.schema.config.public_base_url.description"),
            format: Some("uri".to_string()),
            secret: false,
        },
    );
    insert(
        "default_chat_id",
        false,
        SchemaIr::String {
            title: i18n("telegram.schema.config.default_chat_id.title"),
            description: i18n("telegram.schema.config.default_chat_id.description"),
            format: None,
            secret: false,
        },
    );
    insert(
        "api_base_url",
        true,
        SchemaIr::String {
            title: i18n("telegram.schema.config.api_base_url.title"),
            description: i18n("telegram.schema.config.api_base_url.description"),
            format: Some("uri".to_string()),
            secret: false,
        },
    );
    insert(
        "bot_token",
        false,
        SchemaIr::String {
            title: i18n("telegram.schema.config.bot_token.title"),
            description: i18n("telegram.schema.config.bot_token.description"),
            format: None,
            secret: true,
        },
    );
    SchemaIr::Object {
        title: i18n("telegram.schema.config.title"),
        description: i18n("telegram.schema.config.description"),
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

fn qa_q(key: &str, text: &str, required: bool) -> QaQuestionSpec {
    QaQuestionSpec {
        key: key.to_string(),
        text: i18n(text),
        required,
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
        public_base_url: String::new(),
        default_chat_id: None,
        api_base_url: DEFAULT_API_BASE.to_string(),
        bot_token: None,
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
    if config.api_base_url.trim().is_empty() {
        return Err("invalid config: api_base_url cannot be empty".to_string());
    }
    if !(config.api_base_url.starts_with("http://") || config.api_base_url.starts_with("https://"))
    {
        return Err("invalid config: api_base_url must be an absolute URL".to_string());
    }
    Ok(())
}

fn handle_send(input_json: &[u8]) -> Vec<u8> {
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
            Err(err) => return json_bytes(&json!({"ok": false, "error": err})),
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
        cfg.default_chat_id.clone().map(|chat| Destination {
            id: chat,
            kind: Some("chat".into()),
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
    let kind = destination.kind.as_deref().unwrap_or("chat");
    if kind != "chat" && !kind.is_empty() {
        return json_bytes(&json!({
            "ok": false,
            "error": format!("unsupported destination kind: {kind}")
        }));
    }

    let token = match get_bot_token(&cfg) {
        Ok(s) => s,
        Err(err) => return json_bytes(&json!({"ok": false, "error": err})),
    };

    let api_base = cfg
        .api_base_url
        .clone()
        .unwrap_or_else(|| DEFAULT_API_BASE.to_string());
    let url = format!("{api_base}/bot{token}/sendMessage");
    let payload = json!({ "chat_id": dest_id.clone(), "text": text });
    let request = client::Request {
        method: "POST".to_string(),
        url,
        headers: vec![("Content-Type".into(), "application/json".into())],
        body: Some(serde_json::to_vec(&payload).unwrap_or_else(|_| b"{}".to_vec())),
    };

    let resp = match client::send(&request, None, None) {
        Ok(resp) => resp,
        Err(err) => {
            return json_bytes(&json!({
                "ok": false,
                "error": format!("transport error: {}", err.message),
            }));
        }
    };

    if resp.status < 200 || resp.status >= 300 {
        return json_bytes(&json!({
            "ok": false,
            "error": format!("telegram returned status {}", resp.status),
        }));
    }

    let body = resp.body.unwrap_or_default();
    let body_json: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let (message_id, provider_message_id) = extract_ids(&body_json);

    json_bytes(&json!({
        "ok": true,
        "status": "sent",
        "provider_type": PROVIDER_TYPE,
        "public_base_url": cfg.public_base_url,
        "message_id": message_id,
        "provider_message_id": provider_message_id,
        "response": body_json
    }))
}

fn handle_reply(input_json: &[u8]) -> Vec<u8> {
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

    let text = match parsed
        .get("text")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
    {
        Some(t) if !t.is_empty() => t,
        _ => return json_bytes(&json!({"ok": false, "error": "text required"})),
    };

    let chat_id = match parsed
        .get("chat_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| cfg.default_chat_id.clone())
    {
        Some(chat) if !chat.is_empty() => chat,
        _ => return json_bytes(&json!({"ok": false, "error": "chat_id required"})),
    };

    let reply_to = parsed
        .get("reply_to_id")
        .or_else(|| parsed.get("thread_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if reply_to.is_empty() {
        return json_bytes(&json!({"ok": false, "error": "reply_to_id or thread_id required"}));
    }

    let token = match get_bot_token(&cfg) {
        Ok(s) => s,
        Err(err) => return json_bytes(&json!({"ok": false, "error": err})),
    };

    let api_base = cfg
        .api_base_url
        .clone()
        .unwrap_or_else(|| DEFAULT_API_BASE.to_string());
    let url = format!("{api_base}/bot{token}/sendMessage");
    let payload = json!({
        "chat_id": chat_id,
        "text": text,
        "reply_to_message_id": reply_to
    });
    let request = client::Request {
        method: "POST".to_string(),
        url,
        headers: vec![("Content-Type".to_string(), "application/json".to_string())],
        body: Some(serde_json::to_vec(&payload).unwrap_or_else(|_| b"{}".to_vec())),
    };

    let resp = match client::send(&request, None, None) {
        Ok(resp) => resp,
        Err(err) => {
            return json_bytes(&json!({
                "ok": false,
                "error": format!("transport error: {}", err.message),
            }));
        }
    };

    if resp.status < 200 || resp.status >= 300 {
        return json_bytes(&json!({
            "ok": false,
            "error": format!("telegram returned status {}", resp.status),
        }));
    }

    let body_bytes = resp.body.unwrap_or_default();
    let body_json: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    let (message_id, provider_message_id) = extract_ids(&body_json);

    json_bytes(&json!({
        "ok": true,
        "status": "replied",
        "provider_type": PROVIDER_TYPE,
        "public_base_url": cfg.public_base_url,
        "message_id": message_id,
        "provider_message_id": provider_message_id,
        "response": body_json
    }))
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
    let message = body_val.get("message").cloned().unwrap_or(Value::Null);
    let text = extract_message_text(&message);
    let chat_id = extract_chat_id(&message);
    let from = extract_from_user(&message);
    let envelope = build_telegram_envelope(text.clone(), chat_id.clone(), from.clone());
    let normalized = json!({
        "ok": true,
        "event": body_val,
        "message": message,
        "chat_id": chat_id,
        "from": from
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
    match std::panic::catch_unwind(|| render_plan_inner(input_json)) {
        Ok(result) => result,
        Err(err) => {
            eprintln!("telegram render_plan panic: {err:?}");
            std::panic::resume_unwind(err);
        }
    }
}

fn render_plan_inner(input_json: &[u8]) -> Vec<u8> {
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
                .filter(|text| !text.trim().is_empty())
        })
        .unwrap_or_else(|| "telegram message".to_string());
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
        "debug": {},
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
    let mut envelope = encode_in.message;
    let has_text = envelope
        .text
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .is_some();
    if !has_text {
        envelope.text = Some("universal telegram payload".to_string());
    }
    let body_bytes = serde_json::to_vec(&envelope).unwrap_or_else(|_| b"{}".to_vec());
    let payload = ProviderPayloadV1 {
        content_type: "application/json".to_string(),
        body_b64: STANDARD.encode(&body_bytes),
        metadata: BTreeMap::new(),
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
    let payload_bytes = match STANDARD.decode(&send_in.payload.body_b64) {
        Ok(bytes) => bytes,
        Err(err) => {
            return send_payload_error(&format!("payload decode failed: {err}"), false);
        }
    };
    let payload: Value = serde_json::from_slice(&payload_bytes).unwrap_or(Value::Null);
    match forward_send_payload(&payload) {
        Ok(_) => send_payload_success(),
        Err(err) => send_payload_error(&err, false),
    }
}

fn forward_send_payload(payload: &Value) -> Result<(), String> {
    let payload_bytes =
        serde_json::to_vec(payload).map_err(|err| format!("serialize failed: {err}"))?;
    let result = handle_send(&payload_bytes);
    let result_value: Value =
        serde_json::from_slice(&result).map_err(|err| format!("parse send result: {err}"))?;
    let ok = result_value
        .get("ok")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if ok {
        Ok(())
    } else {
        let message = result_value
            .get("error")
            .and_then(Value::as_str)
            .map(|s| s.to_string())
            .unwrap_or_else(|| "send_payload failed".to_string());
        Err(message)
    }
}

fn build_telegram_envelope(
    text: String,
    chat_id: Option<String>,
    from: Option<String>,
) -> ChannelMessageEnvelope {
    let env = EnvId::try_from("default").expect("env id");
    let tenant = TenantId::try_from("default").expect("tenant id");
    let mut metadata = MessageMetadata::new();
    metadata.insert("universal".to_string(), "true".to_string());
    if let Some(chat) = &chat_id {
        metadata.insert("chat_id".to_string(), chat.clone());
    }
    if let Some(sender) = &from {
        metadata.insert("from".to_string(), sender.clone());
    }
    let channel = "telegram".to_string();
    let sender = from.map(|id| Actor {
        id,
        kind: Some("user".into()),
    });
    ChannelMessageEnvelope {
        id: format!("telegram-{channel}"),
        tenant: TenantCtx::new(env.clone(), tenant.clone()),
        channel: channel.clone(),
        session_id: chat_id.clone().unwrap_or_else(|| "telegram".to_string()),
        reply_scope: None,
        from: sender,
        to: Vec::new(),
        correlation_id: None,
        text: Some(text),
        attachments: Vec::new(),
        metadata,
    }
}

fn build_synthetic_envelope(
    parsed: &Value,
    cfg: &ProviderConfig,
) -> Result<ChannelMessageEnvelope, String> {
    let text = parsed
        .get("text")
        .and_then(|value| value.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| "text required".to_string())?;

    let chat_id = parsed
        .get("chat_id")
        .and_then(|value| value.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| cfg.default_chat_id.clone())
        .ok_or_else(|| "chat_id required".to_string())?;

    let env = EnvId::try_from("synthetic").expect("manual env id");
    let tenant = TenantId::try_from("synthetic").expect("manual tenant id");
    let mut metadata = MessageMetadata::new();
    metadata.insert("chat_id".to_string(), chat_id.clone());
    metadata.insert("synthetic".to_string(), "true".to_string());

    let destination = Destination {
        id: chat_id.clone(),
        kind: Some("chat".to_string()),
    };

    Ok(ChannelMessageEnvelope {
        id: format!("synthetic-telegram-{chat_id}"),
        tenant: TenantCtx::new(env, tenant),
        channel: chat_id.clone(),
        session_id: chat_id.clone(),
        reply_scope: None,
        from: None,
        to: vec![destination],
        correlation_id: None,
        text: Some(text),
        attachments: Vec::new(),
        metadata,
    })
}

fn extract_message_text(value: &Value) -> String {
    value
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn extract_chat_id(value: &Value) -> Option<String> {
    value
        .get("chat")
        .and_then(|chat| chat.get("id"))
        .and_then(Value::as_i64)
        .map(|id| id.to_string())
}

fn extract_from_user(value: &Value) -> Option<String> {
    value
        .get("from")
        .and_then(|from| from.get("id"))
        .and_then(Value::as_i64)
        .map(|id| id.to_string())
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

fn extract_ids(body: &Value) -> (String, String) {
    let message_id = body
        .get("result")
        .and_then(|v| v.get("message_id"))
        .map(|val| match val {
            Value::Number(num) => num.to_string(),
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .unwrap_or_else(|| "dummy-message-id".into());
    let provider_message_id = format!("tg:{message_id}");
    (message_id, provider_message_id)
}

fn parse_config_value(val: &Value) -> Result<ProviderConfig, String> {
    let cfg = serde_json::from_value::<ProviderConfig>(val.clone())
        .map_err(|e| format!("invalid config: {e}"))?;
    validate_provider_config(cfg)
}

#[cfg(test)]
fn parse_config_bytes(bytes: &[u8]) -> Result<ProviderConfig, String> {
    let cfg = serde_json::from_slice::<ProviderConfig>(bytes)
        .map_err(|e| format!("invalid config: {e}"))?;
    validate_provider_config(cfg)
}

fn load_config(input: &Value) -> Result<ProviderConfig, String> {
    if let Some(cfg) = input.get("config") {
        return parse_config_value(cfg);
    }
    let mut partial = serde_json::Map::new();
    if let Some(v) = input.get("enabled") {
        partial.insert("enabled".into(), v.clone());
    }
    if let Some(v) = input.get("public_base_url") {
        partial.insert("public_base_url".into(), v.clone());
    }
    if let Some(v) = input.get("default_chat_id") {
        partial.insert("default_chat_id".into(), v.clone());
    }
    if let Some(v) = input.get("api_base_url") {
        partial.insert("api_base_url".into(), v.clone());
    }
    if let Some(v) = input.get("bot_token") {
        partial.insert("bot_token".into(), v.clone());
    }
    if !partial.is_empty() {
        return parse_config_value(&Value::Object(partial));
    }

    Ok(ProviderConfig {
        enabled: true,
        public_base_url: "https://invalid.local".to_string(),
        default_chat_id: None,
        api_base_url: Some(DEFAULT_API_BASE.to_string()),
        bot_token: None,
    })
}

fn validate_provider_config(cfg: ProviderConfig) -> Result<ProviderConfig, String> {
    if cfg.public_base_url.trim().is_empty() {
        return Err("invalid config: public_base_url cannot be empty".to_string());
    }
    Ok(cfg)
}

fn get_bot_token(cfg: &ProviderConfig) -> Result<String, String> {
    if let Some(token) = cfg.bot_token.clone() {
        let token = token.trim().to_string();
        if !token.is_empty() {
            return Ok(token);
        }
    }
    match secrets_store::get(TOKEN_SECRET) {
        Ok(Some(bytes)) => String::from_utf8(bytes).map_err(|_| "bot token not utf-8".to_string()),
        Ok(None) => Err(format!(
            "missing bot_token (config or secret: {TOKEN_SECRET})"
        )),
        Err(e) => Err(format!("secret store error: {e:?}")),
    }
}

fn json_bytes<T: serde::Serialize>(value: &T) -> Vec<u8> {
    serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn load_config_prefers_nested_config() {
        let input = json!({
            "config": {
                "enabled": true,
                "public_base_url": "https://example.com",
                "default_chat_id": "abc",
                "api_base_url": "https://api.telegram.org"
            },
        });
        let cfg = load_config(&input).expect("config");
        assert_eq!(cfg.default_chat_id.as_deref(), Some("abc"));
    }

    #[test]
    fn parse_config_requires_new_fields() {
        let cfg = br#"{"enabled":true,"public_base_url":"https://example.com","api_base_url":"https://api.telegram.org"}"#;
        let parsed = parse_config_bytes(cfg).expect("config");
        assert!(parsed.enabled);
    }

    #[test]
    fn parse_config_bytes_rejects_unknown_fields() {
        let cfg = br#"{ "enabled": true, "public_base_url": "https://example.com", "api_base_url": "https://api.telegram.org", "default_chat_id": "abc", "unknown": "field" }"#;
        let err = parse_config_bytes(cfg).expect_err("should fail");
        assert!(err.contains("unknown field"));
    }

    #[test]
    fn extract_ids_handles_strings() {
        let body = json!({"result": {"message_id": "42"}});
        let (id, provider) = extract_ids(&body);
        assert_eq!(id, "42");
        assert_eq!(provider, "tg:42");
    }

    #[test]
    fn schema_hash_is_stable() {
        let describe = build_describe_payload();
        assert_eq!(
            describe.schema_hash,
            "be8773298b0229af6f641e622417c198970df42bac96cc560dd44569c4034328"
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
        assert_eq!(keys, vec!["public_base_url"]);
    }

    #[test]
    fn apply_answers_upgrade_preserves_unspecified_fields() {
        use bindings::exports::greentic::component::qa::Guest as QaGuest;
        use bindings::exports::greentic::component::qa::Mode;
        let answers = json!({
            "existing_config": {
                "enabled": true,
                "public_base_url": "https://example.com",
                "default_chat_id": "123",
                "api_base_url": "https://api.telegram.org",
                "bot_token": "token-a"
            },
            "default_chat_id": "456"
        });
        let out =
            <Component as QaGuest>::apply_answers(Mode::Upgrade, canonical_cbor_bytes(&answers));
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
            config.get("default_chat_id"),
            Some(&Value::String("456".to_string()))
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
