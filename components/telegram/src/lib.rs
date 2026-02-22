#![allow(unsafe_op_in_unsafe_fn)]

use bindings::greentic::http::http_client as client;
use bindings::greentic::secrets_store::secrets_store;
#[cfg(not(test))]
use bindings::greentic::telemetry::logger_api;
use provider_common::ProviderError;
use provider_common::component_v0_6::{
    DescribePayload, I18nText, OperationDescriptor, QaQuestionSpec, QaSpec, RedactionRule,
    SchemaField, SchemaIr, canonical_cbor_bytes, decode_cbor, schema_hash,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[allow(clippy::too_many_arguments)]
mod bindings {
    wit_bindgen::generate!({ path: "wit/telegram", world: "component-v0-v6-v0", generate_all });
}

const PROVIDER_ID: &str = "telegram";
const WORLD_ID: &str = "component-v0-v6-v0";
const DEFAULT_API_BASE: &str = "https://api.telegram.org";
const DEFAULT_PARSE_MODE: &str = "HTML";
const DEFAULT_BOT_TOKEN_SECRET: &str = "TELEGRAM_BOT_TOKEN";
const MAX_TEXT_LEN: usize = 4000;
const CALLBACK_DATA_MAX_BYTES: usize = 64;
const MAX_BUTTONS_PER_ROW: usize = 5;
const MAX_BUTTON_ROWS: usize = 8;

const I18N_KEYS: &[&str] = &[
    "telegram.op.run.title",
    "telegram.op.run.description",
    "telegram.op.send.title",
    "telegram.op.send.description",
    "telegram.op.ingest_http.title",
    "telegram.op.ingest_http.description",
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
    "telegram.schema.config.api_base_url.title",
    "telegram.schema.config.api_base_url.description",
    "telegram.schema.config.default_chat_id.title",
    "telegram.schema.config.default_chat_id.description",
    "telegram.schema.config.bot_token.title",
    "telegram.schema.config.bot_token.description",
    "telegram.schema.config.parse_mode.title",
    "telegram.schema.config.parse_mode.description",
    "telegram.qa.default.title",
    "telegram.qa.setup.title",
    "telegram.qa.upgrade.title",
    "telegram.qa.remove.title",
    "telegram.qa.setup.enabled",
    "telegram.qa.setup.public_base_url",
    "telegram.qa.setup.api_base_url",
    "telegram.qa.setup.default_chat_id",
    "telegram.qa.setup.bot_token",
    "telegram.qa.setup.parse_mode",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderConfig {
    #[serde(default = "default_enabled")]
    enabled: bool,
    public_base_url: String,
    #[serde(default = "default_api_base")]
    api_base_url: String,
    #[serde(default)]
    default_chat_id: Option<String>,
    #[serde(default)]
    bot_token: Option<String>,
    #[serde(default = "default_parse_mode")]
    parse_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ApplyAnswersResult {
    ok: bool,
    config: Option<ProviderConfig>,
    error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ButtonOpenUrl {
    text: String,
    url: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ButtonPostback {
    text: String,
    callback_data: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ButtonIn {
    OpenUrl(ButtonOpenUrl),
    Postback(ButtonPostback),
}

struct Component;

impl bindings::exports::greentic::component::descriptor::Guest for Component {
    fn describe() -> Vec<u8> {
        canonical_cbor_bytes(&build_describe_payload())
    }
}

impl bindings::exports::greentic::component::runtime::Guest for Component {
    fn invoke(op: String, input_cbor: Vec<u8>) -> Vec<u8> {
        let input: Value = match decode_cbor(&input_cbor) {
            Ok(value) => value,
            Err(err) => {
                return canonical_cbor_bytes(
                    &json!({"ok": false, "error": format!("invalid input cbor: {err}")}),
                );
            }
        };

        let normalized_op = if op == "run" { "send" } else { op.as_str() };
        let output = match normalized_op {
            "send" => handle_send(&input),
            "ingest_http" => handle_ingest_http(&input),
            "encode" => handle_encode(&input),
            "send_payload" => handle_send_payload(&input),
            other => json!({"ok": false, "error": format!("unsupported op: {other}")}),
        };

        canonical_cbor_bytes(&output)
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
                    error: Some(format!("invalid answers cbor: {err}")),
                });
            }
        };

        if mode == bindings::exports::greentic::component::qa::Mode::Setup {
            let cfg = ProviderConfig {
                enabled: answers
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
                public_base_url: answers
                    .get("public_base_url")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string(),
                api_base_url: answers
                    .get("api_base_url")
                    .and_then(Value::as_str)
                    .unwrap_or(DEFAULT_API_BASE)
                    .trim()
                    .to_string(),
                default_chat_id: answers
                    .get("default_chat_id")
                    .and_then(Value::as_str)
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
                bot_token: answers
                    .get("bot_token")
                    .and_then(Value::as_str)
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
                parse_mode: answers
                    .get("parse_mode")
                    .and_then(Value::as_str)
                    .unwrap_or(DEFAULT_PARSE_MODE)
                    .trim()
                    .to_string(),
            };

            if let Err(err) = validate_provider_config(&cfg) {
                return canonical_cbor_bytes(&ApplyAnswersResult {
                    ok: false,
                    config: None,
                    error: Some(err),
                });
            }

            return canonical_cbor_bytes(&ApplyAnswersResult {
                ok: true,
                config: Some(cfg),
                error: None,
            });
        }

        canonical_cbor_bytes(&ApplyAnswersResult {
            ok: true,
            config: None,
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
        let mut messages = serde_json::Map::new();
        for key in I18N_KEYS {
            messages.insert((*key).to_string(), Value::String((*key).to_string()));
        }
        canonical_cbor_bytes(&json!({"locale": locale, "messages": Value::Object(messages)}))
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
                "ingest_http",
                "telegram.op.ingest_http.title",
                "telegram.op.ingest_http.description",
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
            description: None,
            questions: Vec::new(),
            defaults: Default::default(),
        },
        Mode::Setup => QaSpec {
            mode: "setup".to_string(),
            title: i18n("telegram.qa.setup.title"),
            description: None,
            questions: vec![
                qa_q("enabled", "telegram.qa.setup.enabled", true),
                qa_q("public_base_url", "telegram.qa.setup.public_base_url", true),
                qa_q("api_base_url", "telegram.qa.setup.api_base_url", true),
                qa_q(
                    "default_chat_id",
                    "telegram.qa.setup.default_chat_id",
                    false,
                ),
                qa_q("bot_token", "telegram.qa.setup.bot_token", false),
                qa_q("parse_mode", "telegram.qa.setup.parse_mode", true),
            ],
            defaults: Default::default(),
        },
        Mode::Upgrade => QaSpec {
            mode: "upgrade".to_string(),
            title: i18n("telegram.qa.upgrade.title"),
            description: None,
            questions: Vec::new(),
            defaults: Default::default(),
        },
        Mode::Remove => QaSpec {
            mode: "remove".to_string(),
            title: i18n("telegram.qa.remove.title"),
            description: None,
            questions: Vec::new(),
            defaults: Default::default(),
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
    fields.insert(
        "enabled".to_string(),
        SchemaField {
            required: true,
            schema: SchemaIr::Bool {
                title: i18n("telegram.schema.config.enabled.title"),
                description: i18n("telegram.schema.config.enabled.description"),
            },
        },
    );
    fields.insert(
        "public_base_url".to_string(),
        SchemaField {
            required: true,
            schema: SchemaIr::String {
                title: i18n("telegram.schema.config.public_base_url.title"),
                description: i18n("telegram.schema.config.public_base_url.description"),
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
                title: i18n("telegram.schema.config.api_base_url.title"),
                description: i18n("telegram.schema.config.api_base_url.description"),
                format: Some("uri".to_string()),
                secret: false,
            },
        },
    );
    fields.insert(
        "default_chat_id".to_string(),
        SchemaField {
            required: false,
            schema: SchemaIr::String {
                title: i18n("telegram.schema.config.default_chat_id.title"),
                description: i18n("telegram.schema.config.default_chat_id.description"),
                format: None,
                secret: false,
            },
        },
    );
    fields.insert(
        "bot_token".to_string(),
        SchemaField {
            required: false,
            schema: SchemaIr::String {
                title: i18n("telegram.schema.config.bot_token.title"),
                description: i18n("telegram.schema.config.bot_token.description"),
                format: None,
                secret: true,
            },
        },
    );
    fields.insert(
        "parse_mode".to_string(),
        SchemaField {
            required: true,
            schema: SchemaIr::String {
                title: i18n("telegram.schema.config.parse_mode.title"),
                description: i18n("telegram.schema.config.parse_mode.description"),
                format: None,
                secret: false,
            },
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
        id: key.to_string(),
        label: i18n(text),
        help: None,
        error: None,
        kind: provider_common::component_v0_6::QuestionKind::Text,
        required,
        default: None,
    }
}

fn i18n(key: &str) -> I18nText {
    I18nText {
        key: key.to_string(),
    }
}

fn handle_send(input: &Value) -> Value {
    let cfg = match load_config(input) {
        Ok(cfg) => cfg,
        Err(err) => return json!({"ok": false, "error": err}),
    };
    if !cfg.enabled {
        return json!({"ok": false, "error": "provider disabled by config"});
    }

    let text = input
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| input.get("text").and_then(Value::as_str))
        .map(str::trim)
        .unwrap_or("");
    if text.is_empty() {
        return json!({"ok": false, "error": "missing message"});
    }

    let chat_id = input
        .get("chat_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .or_else(|| cfg.default_chat_id.clone())
        .ok_or_else(|| "missing chat_id and no default_chat_id configured".to_string());

    let chat_id = match chat_id {
        Ok(v) => v,
        Err(err) => return json!({"ok": false, "error": err}),
    };

    let token = match cfg.bot_token.clone() {
        Some(token) if !token.trim().is_empty() => token,
        _ => match get_secret_string(DEFAULT_BOT_TOKEN_SECRET) {
            Ok(v) => v,
            Err(err) => return json!({"ok": false, "error": err}),
        },
    };

    let render = render_payload(input, &cfg, &chat_id, text);

    if input
        .get("dry_run")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return json!({
            "ok": true,
            "message_id": message_id(&render.payload),
            "dry_run": true,
            "payload": render.payload,
            "warnings": render.warnings,
        });
    }

    let req = client::Request {
        method: "POST".into(),
        url: format!(
            "{}/bot{}/sendMessage",
            cfg.api_base_url.trim_end_matches('/'),
            token
        ),
        headers: vec![("Content-Type".into(), "application/json".into())],
        body: serde_json::to_vec(&render.payload).ok(),
    };

    let options = client::RequestOptions {
        timeout_ms: None,
        allow_insecure: Some(false),
        follow_redirects: None,
    };

    match http_send(&req, &options) {
        Ok(resp) if (200..300).contains(&resp.status) => {
            log_if_enabled("send_message_success");
            let mid = resp
                .body
                .and_then(|body| serde_json::from_slice::<Value>(&body).ok())
                .and_then(|v| {
                    v.get("result")
                        .and_then(Value::as_object)
                        .and_then(|obj| obj.get("message_id"))
                        .and_then(Value::as_i64)
                        .map(|v| v.to_string())
                })
                .unwrap_or_else(|| message_id(&render.payload));
            json!({"ok": true, "message_id": mid, "warnings": render.warnings})
        }
        Ok(resp) => {
            json!({"ok": false, "error": format!("transport error: telegram returned status {}", resp.status)})
        }
        Err(err) => {
            json!({"ok": false, "error": format!("transport error: {} ({})", err.message, err.code)})
        }
    }
}

fn handle_ingest_http(input: &Value) -> Value {
    let body = input.get("body").cloned().unwrap_or_else(|| json!({}));
    json!({"ok": true, "event": body})
}

fn handle_encode(input: &Value) -> Value {
    let text = input
        .get("summary_text")
        .and_then(Value::as_str)
        .or_else(|| input.get("message").and_then(Value::as_str))
        .unwrap_or_default();
    let chat_id = input
        .get("chat_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let cfg = ProviderConfig {
        enabled: true,
        public_base_url: "https://example.invalid".to_string(),
        api_base_url: default_api_base(),
        default_chat_id: None,
        bot_token: None,
        parse_mode: default_parse_mode(),
    };
    let rendered = render_payload(input, &cfg, chat_id, text);

    json!({
        "ok": true,
        "payload": {
            "content_type": "application/json",
            "body": rendered.payload,
            "metadata_json": null
        },
        "warnings": rendered.warnings
    })
}

fn handle_send_payload(_input: &Value) -> Value {
    json!({"ok": true, "retryable": false, "message": null})
}

struct RenderedPayload {
    payload: Value,
    warnings: Vec<String>,
}

fn render_payload(
    input: &Value,
    cfg: &ProviderConfig,
    chat_id: &str,
    text: &str,
) -> RenderedPayload {
    let mut warnings = Vec::new();
    let escaped = htmlescape::encode_minimal(text);
    let mut text_bytes = escaped.as_bytes().to_vec();
    if text_bytes.len() > MAX_TEXT_LEN {
        text_bytes.truncate(MAX_TEXT_LEN);
        warnings.push(format!(
            "text truncated to {} bytes to satisfy limit",
            MAX_TEXT_LEN
        ));
    }
    let text = String::from_utf8(text_bytes).unwrap_or_default();

    let mut payload = json!({
        "chat_id": chat_id,
        "text": text,
        "parse_mode": cfg.parse_mode,
    });

    if let Some(thread_id) = input.get("message_thread_id").and_then(Value::as_u64) {
        payload["message_thread_id"] = json!(thread_id);
    }
    if let Some(reply_to) = input.get("reply_to_message_id").and_then(Value::as_u64) {
        payload["reply_to_message_id"] = json!(reply_to);
    }

    let keyboard = build_inline_keyboard(input, &mut warnings);
    if !keyboard.is_empty() {
        payload["reply_markup"] = json!({"inline_keyboard": keyboard});
    }

    RenderedPayload { payload, warnings }
}

fn build_inline_keyboard(input: &Value, warnings: &mut Vec<String>) -> Vec<Vec<Value>> {
    let Some(buttons_val) = input.get("buttons").and_then(Value::as_array) else {
        return Vec::new();
    };

    let mut rows: Vec<Vec<Value>> = Vec::new();
    let mut current: Vec<Value> = Vec::new();

    for raw in buttons_val {
        if rows.len() >= MAX_BUTTON_ROWS {
            warnings.push("button rows exceeded max; dropping remaining buttons".to_string());
            break;
        }

        if current.len() >= MAX_BUTTONS_PER_ROW {
            rows.push(current);
            current = Vec::new();
        }

        match serde_json::from_value::<ButtonIn>(raw.clone()) {
            Ok(ButtonIn::OpenUrl(v)) => {
                current.push(json!({"text": v.text, "url": v.url}));
            }
            Ok(ButtonIn::Postback(v)) => {
                let mut data = v.callback_data.into_bytes();
                if data.len() > CALLBACK_DATA_MAX_BYTES {
                    data.truncate(CALLBACK_DATA_MAX_BYTES);
                    warnings.push(format!(
                        "callback_data truncated to {} bytes",
                        CALLBACK_DATA_MAX_BYTES
                    ));
                }
                let callback_data = String::from_utf8(data).unwrap_or_default();
                current.push(json!({"text": v.text, "callback_data": callback_data}));
            }
            Err(_) => warnings.push("invalid button ignored".to_string()),
        }
    }

    if !current.is_empty() {
        rows.push(current);
    }
    rows
}

fn message_id(payload: &Value) -> String {
    let bytes = serde_json::to_vec(payload).unwrap_or_default();
    provider_common::component_v0_6::sha256_hex(&bytes)
}

fn default_enabled() -> bool {
    true
}

fn default_api_base() -> String {
    DEFAULT_API_BASE.to_string()
}

fn default_parse_mode() -> String {
    DEFAULT_PARSE_MODE.to_string()
}

fn load_config(input: &Value) -> Result<ProviderConfig, String> {
    let candidate = input
        .get("config")
        .cloned()
        .unwrap_or_else(|| input.clone());
    let cfg: ProviderConfig = serde_json::from_value(candidate)
        .map_err(|err| format!("invalid provider config: {err}"))?;
    validate_provider_config(&cfg)?;
    Ok(cfg)
}

fn validate_provider_config(cfg: &ProviderConfig) -> Result<(), String> {
    if cfg.public_base_url.trim().is_empty() {
        return Err("public_base_url must be non-empty".to_string());
    }
    if cfg.api_base_url.trim().is_empty() {
        return Err("api_base_url must be non-empty".to_string());
    }
    if cfg.parse_mode.trim().is_empty() {
        return Err("parse_mode must be non-empty".to_string());
    }
    Ok(())
}

fn get_secret_string(key: &str) -> Result<String, String> {
    match secrets_get(key) {
        Ok(Some(bytes)) => String::from_utf8(bytes).map_err(|_| "secret not valid utf-8".into()),
        Ok(None) => Err(missing_secret_error(key)),
        Err(error) => Err(secret_error_message(key, error)),
    }
}

fn missing_secret_error(name: &str) -> String {
    serde_json::to_string(&ProviderError::missing_secret(name))
        .unwrap_or_else(|_| format!("missing secret: {name}"))
}

fn secret_error_message(key: &str, error: secrets_store::SecretsError) -> String {
    match error {
        secrets_store::SecretsError::NotFound => missing_secret_error(key),
        secrets_store::SecretsError::Denied => "secret access denied".into(),
        secrets_store::SecretsError::InvalidKey => "secret key invalid".into(),
        secrets_store::SecretsError::Internal => "secret lookup failed".into(),
    }
}

fn log_if_enabled(event: &str) {
    #[cfg(test)]
    {
        let _ = event;
    }

    #[cfg(not(test))]
    let span = logger_api::SpanContext {
        tenant: "tenant".into(),
        session_id: None,
        flow_id: "provider-runtime".into(),
        node_id: None,
        provider: "telegram".into(),
        start_ms: None,
        end_ms: None,
    };

    #[cfg(not(test))]
    {
        let fields = [("event".to_string(), event.to_string())];
        let _ = logger_api::log(&span, &fields, None);
    }
}

fn secrets_get(key: &str) -> Result<Option<Vec<u8>>, secrets_store::SecretsError> {
    #[cfg(test)]
    {
        secrets_get_test(key)
    }
    #[cfg(not(test))]
    {
        secrets_store::get(key)
    }
}

fn http_send(
    req: &client::Request,
    options: &client::RequestOptions,
) -> Result<client::Response, client::HostError> {
    #[cfg(test)]
    {
        http_send_test(req, options)
    }
    #[cfg(not(test))]
    {
        client::send(req, Some(*options), None)
    }
}

#[cfg(test)]
type SecretsGetMock = dyn Fn(&str) -> Result<Option<Vec<u8>>, secrets_store::SecretsError>;

#[cfg(test)]
type HttpSendMock = dyn Fn(
    &client::Request,
    &client::RequestOptions,
) -> Result<client::Response, client::HostError>;

#[cfg(test)]
thread_local! {
    static SECRETS_GET_MOCK: std::cell::RefCell<Option<Box<SecretsGetMock>>> =
        std::cell::RefCell::new(None);
    static HTTP_SEND_MOCK: std::cell::RefCell<Option<Box<HttpSendMock>>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
fn with_secrets_get_mock<F, R>(
    mock: impl Fn(&str) -> Result<Option<Vec<u8>>, secrets_store::SecretsError> + 'static,
    f: F,
) -> R
where
    F: FnOnce() -> R,
{
    SECRETS_GET_MOCK.with(|cell| *cell.borrow_mut() = Some(Box::new(mock)));
    let out = f();
    SECRETS_GET_MOCK.with(|cell| *cell.borrow_mut() = None);
    out
}

#[cfg(test)]
fn with_http_send_mock<F, R>(
    mock: impl Fn(
        &client::Request,
        &client::RequestOptions,
    ) -> Result<client::Response, client::HostError>
    + 'static,
    f: F,
) -> R
where
    F: FnOnce() -> R,
{
    HTTP_SEND_MOCK.with(|cell| *cell.borrow_mut() = Some(Box::new(mock)));
    let out = f();
    HTTP_SEND_MOCK.with(|cell| *cell.borrow_mut() = None);
    out
}

#[cfg(test)]
fn secrets_get_test(key: &str) -> Result<Option<Vec<u8>>, secrets_store::SecretsError> {
    SECRETS_GET_MOCK.with(|cell| match &*cell.borrow() {
        Some(mock) => mock(key),
        None => Ok(None),
    })
}

#[cfg(test)]
fn http_send_test(
    req: &client::Request,
    options: &client::RequestOptions,
) -> Result<client::Response, client::HostError> {
    HTTP_SEND_MOCK.with(|cell| match &*cell.borrow() {
        Some(mock) => mock(req, options),
        None => Err(client::HostError {
            code: "unconfigured".into(),
            message: "http_send_test mock not set".into(),
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn parse_config_rejects_unknown() {
        let value = json!({
            "enabled": true,
            "public_base_url": "https://example.com",
            "api_base_url": "https://api.telegram.org",
            "parse_mode": "HTML",
            "unknown": true
        });
        let err = load_config(&value).unwrap_err();
        assert!(err.contains("unknown field"));
    }

    #[test]
    fn parse_config_requires_new_fields() {
        let value = json!({"enabled": true});
        let err = load_config(&value).unwrap_err();
        assert!(err.contains("public_base_url"));
    }

    #[test]
    fn invoke_run_requires_message() {
        let input = json!({
            "config": {
                "enabled": true,
                "public_base_url": "https://example.com",
                "api_base_url": "https://api.telegram.org",
                "parse_mode": "HTML",
                "default_chat_id": "123",
                "bot_token": "token"
            }
        });
        let out = handle_send(&input);
        assert_eq!(out["ok"], Value::Bool(false));
    }

    #[test]
    fn send_uses_http_mock() {
        let input = json!({
            "message": "hello <world>",
            "chat_id": "123",
            "buttons": [
                {"type": "postback", "text": "A", "callback_data": "data"}
            ],
            "config": {
                "enabled": true,
                "public_base_url": "https://example.com",
                "api_base_url": "https://api.telegram.org",
                "parse_mode": "HTML",
                "bot_token": "token"
            }
        });

        with_http_send_mock(
            |_, _| {
                Ok(client::Response {
                    status: 200,
                    headers: vec![],
                    body: Some(br#"{"ok":true,"result":{"message_id":42}}"#.to_vec()),
                })
            },
            || {
                let out = handle_send(&input);
                assert_eq!(out["ok"], Value::Bool(true));
                assert_eq!(out["message_id"], Value::String("42".to_string()));
            },
        );
    }

    #[test]
    fn secret_fallback_used_when_bot_token_missing() {
        let input = json!({
            "message": "hello",
            "chat_id": "123",
            "config": {
                "enabled": true,
                "public_base_url": "https://example.com",
                "api_base_url": "https://api.telegram.org",
                "parse_mode": "HTML"
            }
        });

        with_secrets_get_mock(
            |name| {
                if name == DEFAULT_BOT_TOKEN_SECRET {
                    Ok(Some(b"token-from-store".to_vec()))
                } else {
                    Ok(None)
                }
            },
            || {
                with_http_send_mock(
                    |req, _| {
                        assert!(req.url.contains("/bottoken-from-store/sendMessage"));
                        Ok(client::Response {
                            status: 200,
                            headers: vec![],
                            body: Some(br#"{"ok":true,"result":{"message_id":7}}"#.to_vec()),
                        })
                    },
                    || {
                        let out = handle_send(&input);
                        assert_eq!(out["ok"], Value::Bool(true));
                    },
                )
            },
        );
    }

    #[test]
    fn schema_hash_is_stable() {
        let describe = build_describe_payload();
        assert_eq!(
            describe.schema_hash,
            "a08fec9d024f4dec10ccd9294524631388fca9f1c253d90b27d459e43de07cbb"
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
}
