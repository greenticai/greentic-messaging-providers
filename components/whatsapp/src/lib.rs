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
    wit_bindgen::generate!({ path: "wit/whatsapp", world: "component-v0-v6-v0", generate_all });
}

const PROVIDER_ID: &str = "whatsapp";
const WORLD_ID: &str = "component-v0-v6-v0";
const DEFAULT_API_BASE: &str = "https://graph.facebook.com";
const DEFAULT_API_VERSION: &str = "v19.0";
const DEFAULT_TOKEN_SECRET: &str = "WHATSAPP_TOKEN";
const DEFAULT_VERIFY_TOKEN_SECRET: &str = "WHATSAPP_VERIFY_TOKEN";

const I18N_KEYS: &[&str] = &[
    "whatsapp.op.run.title",
    "whatsapp.op.run.description",
    "whatsapp.op.send.title",
    "whatsapp.op.send.description",
    "whatsapp.op.ingest_http.title",
    "whatsapp.op.ingest_http.description",
    "whatsapp.op.encode.title",
    "whatsapp.op.encode.description",
    "whatsapp.op.send_payload.title",
    "whatsapp.op.send_payload.description",
    "whatsapp.schema.input.title",
    "whatsapp.schema.input.description",
    "whatsapp.schema.input.message.title",
    "whatsapp.schema.input.message.description",
    "whatsapp.schema.output.title",
    "whatsapp.schema.output.description",
    "whatsapp.schema.output.ok.title",
    "whatsapp.schema.output.ok.description",
    "whatsapp.schema.output.message_id.title",
    "whatsapp.schema.output.message_id.description",
    "whatsapp.schema.config.title",
    "whatsapp.schema.config.description",
    "whatsapp.schema.config.enabled.title",
    "whatsapp.schema.config.enabled.description",
    "whatsapp.schema.config.phone_number_id.title",
    "whatsapp.schema.config.phone_number_id.description",
    "whatsapp.schema.config.public_base_url.title",
    "whatsapp.schema.config.public_base_url.description",
    "whatsapp.schema.config.business_account_id.title",
    "whatsapp.schema.config.business_account_id.description",
    "whatsapp.schema.config.api_base_url.title",
    "whatsapp.schema.config.api_base_url.description",
    "whatsapp.schema.config.api_version.title",
    "whatsapp.schema.config.api_version.description",
    "whatsapp.schema.config.token.title",
    "whatsapp.schema.config.token.description",
    "whatsapp.qa.default.title",
    "whatsapp.qa.setup.title",
    "whatsapp.qa.upgrade.title",
    "whatsapp.qa.remove.title",
    "whatsapp.qa.setup.enabled",
    "whatsapp.qa.setup.phone_number_id",
    "whatsapp.qa.setup.public_base_url",
    "whatsapp.qa.setup.business_account_id",
    "whatsapp.qa.setup.api_base_url",
    "whatsapp.qa.setup.api_version",
    "whatsapp.qa.setup.token",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderConfig {
    #[serde(default = "default_enabled")]
    enabled: bool,
    phone_number_id: String,
    public_base_url: String,
    #[serde(default)]
    business_account_id: Option<String>,
    #[serde(default = "default_api_base")]
    api_base_url: String,
    #[serde(default = "default_api_version")]
    api_version: String,
    #[serde(default)]
    token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ApplyAnswersResult {
    ok: bool,
    config: Option<ProviderConfig>,
    error: Option<String>,
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
                phone_number_id: answers
                    .get("phone_number_id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string(),
                public_base_url: answers
                    .get("public_base_url")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string(),
                business_account_id: answers
                    .get("business_account_id")
                    .and_then(Value::as_str)
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
                api_base_url: answers
                    .get("api_base_url")
                    .and_then(Value::as_str)
                    .unwrap_or(DEFAULT_API_BASE)
                    .trim()
                    .to_string(),
                api_version: answers
                    .get("api_version")
                    .and_then(Value::as_str)
                    .unwrap_or(DEFAULT_API_VERSION)
                    .trim()
                    .to_string(),
                token: answers
                    .get("token")
                    .and_then(Value::as_str)
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
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
                "whatsapp.op.run.title",
                "whatsapp.op.run.description",
            ),
            op(
                "send",
                "whatsapp.op.send.title",
                "whatsapp.op.send.description",
            ),
            op(
                "ingest_http",
                "whatsapp.op.ingest_http.title",
                "whatsapp.op.ingest_http.description",
            ),
            op(
                "encode",
                "whatsapp.op.encode.title",
                "whatsapp.op.encode.description",
            ),
            op(
                "send_payload",
                "whatsapp.op.send_payload.title",
                "whatsapp.op.send_payload.description",
            ),
        ],
        input_schema: input_schema.clone(),
        output_schema: output_schema.clone(),
        config_schema: config_schema.clone(),
        redactions: vec![RedactionRule {
            path: "$.token".to_string(),
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
            title: i18n("whatsapp.qa.default.title"),
            questions: Vec::new(),
        },
        Mode::Setup => QaSpec {
            mode: "setup".to_string(),
            title: i18n("whatsapp.qa.setup.title"),
            questions: vec![
                qa_q("enabled", "whatsapp.qa.setup.enabled", true),
                qa_q("phone_number_id", "whatsapp.qa.setup.phone_number_id", true),
                qa_q("public_base_url", "whatsapp.qa.setup.public_base_url", true),
                qa_q(
                    "business_account_id",
                    "whatsapp.qa.setup.business_account_id",
                    false,
                ),
                qa_q("api_base_url", "whatsapp.qa.setup.api_base_url", true),
                qa_q("api_version", "whatsapp.qa.setup.api_version", true),
                qa_q("token", "whatsapp.qa.setup.token", false),
            ],
        },
        Mode::Upgrade => QaSpec {
            mode: "upgrade".to_string(),
            title: i18n("whatsapp.qa.upgrade.title"),
            questions: Vec::new(),
        },
        Mode::Remove => QaSpec {
            mode: "remove".to_string(),
            title: i18n("whatsapp.qa.remove.title"),
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
                title: i18n("whatsapp.schema.input.message.title"),
                description: i18n("whatsapp.schema.input.message.description"),
                format: None,
                secret: false,
            },
        },
    );

    SchemaIr::Object {
        title: i18n("whatsapp.schema.input.title"),
        description: i18n("whatsapp.schema.input.description"),
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
                title: i18n("whatsapp.schema.output.ok.title"),
                description: i18n("whatsapp.schema.output.ok.description"),
            },
        },
    );
    fields.insert(
        "message_id".to_string(),
        SchemaField {
            required: false,
            schema: SchemaIr::String {
                title: i18n("whatsapp.schema.output.message_id.title"),
                description: i18n("whatsapp.schema.output.message_id.description"),
                format: None,
                secret: false,
            },
        },
    );

    SchemaIr::Object {
        title: i18n("whatsapp.schema.output.title"),
        description: i18n("whatsapp.schema.output.description"),
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
                title: i18n("whatsapp.schema.config.enabled.title"),
                description: i18n("whatsapp.schema.config.enabled.description"),
            },
        },
    );
    fields.insert(
        "phone_number_id".to_string(),
        SchemaField {
            required: true,
            schema: SchemaIr::String {
                title: i18n("whatsapp.schema.config.phone_number_id.title"),
                description: i18n("whatsapp.schema.config.phone_number_id.description"),
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
                title: i18n("whatsapp.schema.config.public_base_url.title"),
                description: i18n("whatsapp.schema.config.public_base_url.description"),
                format: Some("uri".to_string()),
                secret: false,
            },
        },
    );
    fields.insert(
        "business_account_id".to_string(),
        SchemaField {
            required: false,
            schema: SchemaIr::String {
                title: i18n("whatsapp.schema.config.business_account_id.title"),
                description: i18n("whatsapp.schema.config.business_account_id.description"),
                format: None,
                secret: false,
            },
        },
    );
    fields.insert(
        "api_base_url".to_string(),
        SchemaField {
            required: true,
            schema: SchemaIr::String {
                title: i18n("whatsapp.schema.config.api_base_url.title"),
                description: i18n("whatsapp.schema.config.api_base_url.description"),
                format: Some("uri".to_string()),
                secret: false,
            },
        },
    );
    fields.insert(
        "api_version".to_string(),
        SchemaField {
            required: true,
            schema: SchemaIr::String {
                title: i18n("whatsapp.schema.config.api_version.title"),
                description: i18n("whatsapp.schema.config.api_version.description"),
                format: None,
                secret: false,
            },
        },
    );
    fields.insert(
        "token".to_string(),
        SchemaField {
            required: false,
            schema: SchemaIr::String {
                title: i18n("whatsapp.schema.config.token.title"),
                description: i18n("whatsapp.schema.config.token.description"),
                format: None,
                secret: true,
            },
        },
    );

    SchemaIr::Object {
        title: i18n("whatsapp.schema.config.title"),
        description: i18n("whatsapp.schema.config.description"),
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

    let to = input
        .get("to")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .unwrap_or_default();
    if to.is_empty() {
        return json!({"ok": false, "error": "missing destination `to`"});
    }

    let token = match cfg.token.clone() {
        Some(token) if !token.trim().is_empty() => token,
        _ => match get_secret_string(DEFAULT_TOKEN_SECRET) {
            Ok(v) => v,
            Err(err) => return json!({"ok": false, "error": err}),
        },
    };

    let payload = json!({
        "messaging_product": "whatsapp",
        "to": to,
        "type": "text",
        "text": {"body": text}
    });

    if input
        .get("dry_run")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return json!({
            "ok": true,
            "message_id": message_id(&payload),
            "dry_run": true,
            "payload": payload
        });
    }

    let req = client::Request {
        method: "POST".into(),
        url: format!(
            "{}/{}/{}/messages",
            cfg.api_base_url.trim_end_matches('/'),
            cfg.api_version,
            cfg.phone_number_id
        ),
        headers: vec![
            ("Content-Type".into(), "application/json".into()),
            ("Authorization".into(), format!("Bearer {}", token)),
        ],
        body: serde_json::to_vec(&payload).ok(),
    };
    let options = client::RequestOptions {
        timeout_ms: None,
        allow_insecure: Some(false),
        follow_redirects: None,
    };

    match http_send(&req, &options) {
        Ok(resp) if (200..300).contains(&resp.status) => {
            log_if_enabled("send_message_success");
            let id = resp
                .body
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .and_then(|v| {
                    v.get("messages")
                        .and_then(Value::as_array)
                        .and_then(|arr| arr.first())
                        .and_then(|v| v.get("id"))
                        .and_then(Value::as_str)
                        .map(ToString::to_string)
                })
                .unwrap_or_else(|| message_id(&payload));
            json!({"ok": true, "message_id": id})
        }
        Ok(resp) => {
            json!({"ok": false, "error": format!("transport error: whatsapp returned status {}", resp.status)})
        }
        Err(err) => {
            json!({"ok": false, "error": format!("transport error: {} ({})", err.message, err.code)})
        }
    }
}

fn handle_ingest_http(input: &Value) -> Value {
    let body = input.get("body").cloned().unwrap_or_else(|| json!({}));

    let verify_token = body
        .get("hub.verify_token")
        .or_else(|| body.get("verify_token"))
        .and_then(Value::as_str);
    if let Some(received) = verify_token {
        match get_secret_string(DEFAULT_VERIFY_TOKEN_SECRET) {
            Ok(expected) if expected == received => {}
            Ok(_) => {
                return json!({"ok": false, "error": "validation error: verify token mismatch"});
            }
            Err(err) => return json!({"ok": false, "error": err}),
        }
    }

    json!({"ok": true, "event": body})
}

fn handle_encode(input: &Value) -> Value {
    let text = input
        .get("summary_text")
        .and_then(Value::as_str)
        .or_else(|| input.get("message").and_then(Value::as_str))
        .unwrap_or_default();
    let payload = json!({
        "messaging_product": "whatsapp",
        "to": input.get("to").and_then(Value::as_str).unwrap_or_default(),
        "type": "text",
        "text": {"body": text}
    });

    json!({
        "ok": true,
        "payload": {
            "content_type": "application/json",
            "body": payload,
            "metadata_json": null
        },
        "warnings": []
    })
}

fn handle_send_payload(_input: &Value) -> Value {
    json!({"ok": true, "retryable": false, "message": null})
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

fn default_api_version() -> String {
    DEFAULT_API_VERSION.to_string()
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
    if cfg.phone_number_id.trim().is_empty() {
        return Err("phone_number_id must be non-empty".to_string());
    }
    if cfg.public_base_url.trim().is_empty() {
        return Err("public_base_url must be non-empty".to_string());
    }
    if cfg.api_base_url.trim().is_empty() {
        return Err("api_base_url must be non-empty".to_string());
    }
    if cfg.api_version.trim().is_empty() {
        return Err("api_version must be non-empty".to_string());
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
        provider: "whatsapp".into(),
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
            "phone_number_id": "123",
            "public_base_url": "https://example.com",
            "api_base_url": "https://graph.facebook.com",
            "api_version": "v19.0",
            "unknown": true
        });
        let err = load_config(&value).unwrap_err();
        assert!(err.contains("unknown field"));
    }

    #[test]
    fn parse_config_requires_new_fields() {
        let value = json!({"enabled": true});
        let err = load_config(&value).unwrap_err();
        assert!(err.contains("phone_number_id") || err.contains("public_base_url"));
    }

    #[test]
    fn invoke_run_requires_message() {
        let input = json!({
            "to": "+12065550123",
            "config": {
                "enabled": true,
                "phone_number_id": "123",
                "public_base_url": "https://example.com",
                "api_base_url": "https://graph.facebook.com",
                "api_version": "v19.0",
                "token": "tok"
            }
        });
        let out = handle_send(&input);
        assert_eq!(out["ok"], Value::Bool(false));
    }

    #[test]
    fn send_uses_http_mock() {
        let input = json!({
            "to": "+12065550123",
            "message": "hello whatsapp",
            "config": {
                "enabled": true,
                "phone_number_id": "123",
                "public_base_url": "https://example.com",
                "api_base_url": "https://graph.facebook.com",
                "api_version": "v19.0",
                "token": "tok"
            }
        });

        with_http_send_mock(
            |_, _| {
                Ok(client::Response {
                    status: 200,
                    headers: vec![],
                    body: Some(br#"{"messages":[{"id":"wamid.1"}]}"#.to_vec()),
                })
            },
            || {
                let out = handle_send(&input);
                assert_eq!(out["ok"], Value::Bool(true));
                assert_eq!(out["message_id"], Value::String("wamid.1".to_string()));
            },
        );
    }

    #[test]
    fn secret_fallback_used_when_token_missing() {
        let input = json!({
            "to": "+12065550123",
            "message": "hello",
            "config": {
                "enabled": true,
                "phone_number_id": "123",
                "public_base_url": "https://example.com",
                "api_base_url": "https://graph.facebook.com",
                "api_version": "v19.0"
            }
        });

        with_secrets_get_mock(
            |name| {
                if name == DEFAULT_TOKEN_SECRET {
                    Ok(Some(b"token-from-store".to_vec()))
                } else {
                    Ok(None)
                }
            },
            || {
                with_http_send_mock(
                    |req, _| {
                        let auth = req
                            .headers
                            .iter()
                            .find(|(k, _)| k == "Authorization")
                            .map(|(_, v)| v.clone())
                            .unwrap_or_default();
                        assert_eq!(auth, "Bearer token-from-store");
                        Ok(client::Response {
                            status: 200,
                            headers: vec![],
                            body: Some(br#"{"messages":[{"id":"wamid.2"}]}"#.to_vec()),
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
            "12fc34242be5488838d7989630baa19d0fbdff69ec3706d8e3b50bb25d2fe45f"
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
}
