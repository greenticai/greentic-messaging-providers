#![allow(unsafe_op_in_unsafe_fn)]

use bindings::greentic::http::http_client as client;
#[cfg(not(test))]
use bindings::greentic::telemetry::logger_api;
use hmac::{Hmac, Mac};
use provider_common::component_v0_6::{
    DescribePayload, I18nText, OperationDescriptor, QaQuestionSpec, QaSpec, RedactionRule,
    SchemaField, SchemaIr, canonical_cbor_bytes, decode_cbor, schema_hash,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[allow(clippy::too_many_arguments)]
mod bindings {
    wit_bindgen::generate!({ path: "wit/slack", world: "component-v0-v6-v0", generate_all });
}

const PROVIDER_ID: &str = "slack";
const WORLD_ID: &str = "component-v0-v6-v0";
const DEFAULT_API_BASE: &str = "https://slack.com/api";

const I18N_KEYS: &[&str] = &[
    "slack.op.run.title",
    "slack.op.run.description",
    "slack.op.send.title",
    "slack.op.send.description",
    "slack.op.ingest_http.title",
    "slack.op.ingest_http.description",
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
    "slack.schema.config.public_base_url.title",
    "slack.schema.config.public_base_url.description",
    "slack.schema.config.api_base_url.title",
    "slack.schema.config.api_base_url.description",
    "slack.schema.config.default_channel.title",
    "slack.schema.config.default_channel.description",
    "slack.schema.config.bot_token.title",
    "slack.schema.config.bot_token.description",
    "slack.schema.config.signing_secret.title",
    "slack.schema.config.signing_secret.description",
    "slack.qa.default.title",
    "slack.qa.setup.title",
    "slack.qa.upgrade.title",
    "slack.qa.remove.title",
    "slack.qa.setup.enabled",
    "slack.qa.setup.public_base_url",
    "slack.qa.setup.api_base_url",
    "slack.qa.setup.default_channel",
    "slack.qa.setup.bot_token",
    "slack.qa.setup.signing_secret",
];

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProviderConfig {
    #[serde(default = "default_enabled")]
    enabled: bool,
    public_base_url: String,
    #[serde(default = "default_api_base")]
    api_base_url: String,
    #[serde(default)]
    default_channel: Option<String>,
    bot_token: String,
    #[serde(default)]
    signing_secret: Option<String>,
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
                default_channel: answers
                    .get("default_channel")
                    .and_then(Value::as_str)
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
                bot_token: answers
                    .get("bot_token")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string(),
                signing_secret: answers
                    .get("signing_secret")
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
            op("run", "slack.op.run.title", "slack.op.run.description"),
            op("send", "slack.op.send.title", "slack.op.send.description"),
            op(
                "ingest_http",
                "slack.op.ingest_http.title",
                "slack.op.ingest_http.description",
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
        redactions: vec![
            RedactionRule {
                path: "$.bot_token".to_string(),
                strategy: "replace".to_string(),
            },
            RedactionRule {
                path: "$.signing_secret".to_string(),
                strategy: "replace".to_string(),
            },
        ],
        schema_hash: schema_hash(&input_schema, &output_schema, &config_schema),
    }
}

fn build_qa_spec(mode: bindings::exports::greentic::component::qa::Mode) -> QaSpec {
    use bindings::exports::greentic::component::qa::Mode;

    match mode {
        Mode::Default => QaSpec {
            mode: "default".to_string(),
            title: i18n("slack.qa.default.title"),
            description: None,
            questions: Vec::new(),
            defaults: Default::default(),
        },
        Mode::Setup => QaSpec {
            mode: "setup".to_string(),
            title: i18n("slack.qa.setup.title"),
            description: None,
            questions: vec![
                qa_q("enabled", "slack.qa.setup.enabled", true),
                qa_q("public_base_url", "slack.qa.setup.public_base_url", true),
                qa_q("api_base_url", "slack.qa.setup.api_base_url", true),
                qa_q("default_channel", "slack.qa.setup.default_channel", false),
                qa_q("bot_token", "slack.qa.setup.bot_token", true),
                qa_q("signing_secret", "slack.qa.setup.signing_secret", false),
            ],
            defaults: Default::default(),
        },
        Mode::Upgrade => QaSpec {
            mode: "upgrade".to_string(),
            title: i18n("slack.qa.upgrade.title"),
            description: None,
            questions: Vec::new(),
            defaults: Default::default(),
        },
        Mode::Remove => QaSpec {
            mode: "remove".to_string(),
            title: i18n("slack.qa.remove.title"),
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
    fields.insert(
        "signing_secret".to_string(),
        SchemaField {
            required: false,
            schema: SchemaIr::String {
                title: i18n("slack.schema.config.signing_secret.title"),
                description: i18n("slack.schema.config.signing_secret.description"),
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

    let message = input
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| input.get("text").and_then(Value::as_str))
        .map(str::trim)
        .unwrap_or("");
    if message.is_empty() {
        return json!({"ok": false, "error": "missing message"});
    }

    let channel = input
        .get("channel")
        .and_then(Value::as_str)
        .or(cfg.default_channel.as_deref())
        .map(str::trim)
        .unwrap_or("");
    if channel.is_empty() {
        return json!({"ok": false, "error": "missing channel and no default_channel configured"});
    }

    let payload = json!({
        "channel": channel,
        "text": message,
        "blocks": [section_md(message)],
    });

    let message_id = payload_hash(&payload);

    if input
        .get("dry_run")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return json!({"ok": true, "message_id": message_id, "dry_run": true, "payload": payload});
    }

    let req = client::Request {
        method: "POST".to_string(),
        url: format!(
            "{}/chat.postMessage",
            cfg.api_base_url.trim_end_matches('/')
        ),
        headers: vec![
            ("Content-Type".into(), "application/json".into()),
            ("Authorization".into(), format!("Bearer {}", cfg.bot_token)),
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
            let ts = resp
                .body
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .and_then(|v| v.get("ts").and_then(Value::as_str).map(ToString::to_string));
            json!({"ok": true, "message_id": ts.unwrap_or(message_id), "status": resp.status})
        }
        Ok(resp) => {
            json!({"ok": false, "error": format!("transport error: slack returned status {}", resp.status)})
        }
        Err(err) => {
            json!({"ok": false, "error": format!("transport error: {} ({})", err.message, err.code)})
        }
    }
}

fn handle_ingest_http(input: &Value) -> Value {
    let cfg = match load_config(input) {
        Ok(cfg) => cfg,
        Err(err) => return json!({"ok": false, "error": err}),
    };

    let headers = input
        .get("headers")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let body_json = input
        .get("body")
        .cloned()
        .unwrap_or(Value::Object(Default::default()));
    let body_text = if let Some(s) = input.get("body_raw").and_then(Value::as_str) {
        s.to_string()
    } else {
        serde_json::to_string(&body_json).unwrap_or_else(|_| "{}".to_string())
    };

    if let Some(secret) = cfg.signing_secret.as_deref()
        && let Err(err) = verify_signature(&headers, &body_text, secret)
    {
        return json!({"ok": false, "error": err.to_string()});
    }

    json!({"ok": true, "event": body_json})
}

fn handle_encode(input: &Value) -> Value {
    let text = input
        .get("summary_text")
        .and_then(Value::as_str)
        .or_else(|| input.get("message").and_then(Value::as_str))
        .unwrap_or_default();
    let channel = input
        .get("channel")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let payload = json!({
        "channel": channel,
        "text": text,
        "blocks": [section_md(text)],
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

fn section_md(text: &str) -> Value {
    json!({"type": "section", "text": {"type": "mrkdwn", "text": text}})
}

fn payload_hash(value: &Value) -> String {
    let mut hasher = Sha256::new();
    hasher.update(serde_json::to_vec(value).unwrap_or_default());
    format!("{:x}", hasher.finalize())
}

fn default_enabled() -> bool {
    true
}

fn default_api_base() -> String {
    DEFAULT_API_BASE.to_string()
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
    if cfg.bot_token.trim().is_empty() {
        return Err("bot_token must be non-empty".to_string());
    }
    if cfg.api_base_url.trim().is_empty() {
        return Err("api_base_url must be non-empty".to_string());
    }
    Ok(())
}

fn verify_signature(
    headers: &serde_json::Map<String, serde_json::Value>,
    body: &str,
    signing_secret: &str,
) -> Result<(), VerificationError> {
    let ts = header_value(headers, "x-slack-request-timestamp")
        .ok_or(VerificationError::MissingTimestamp)?;
    let sig =
        header_value(headers, "x-slack-signature").ok_or(VerificationError::MissingSignature)?;

    let base = format!("v0:{}:{}", ts, body);
    let mut mac = Hmac::<Sha256>::new_from_slice(signing_secret.as_bytes())
        .map_err(|_| VerificationError::InvalidKey)?;
    mac.update(base.as_bytes());
    let computed = mac.finalize().into_bytes();
    let mut hex = String::with_capacity(64);
    for byte in computed {
        use std::fmt::Write;
        write!(&mut hex, "{:02x}", byte).unwrap();
    }
    let expected = format!("v0={}", hex);

    if constant_time_eq(expected.as_bytes(), sig.as_bytes()) {
        Ok(())
    } else {
        Err(VerificationError::SignatureMismatch)
    }
}

fn header_value(
    headers: &serde_json::Map<String, serde_json::Value>,
    name: &str,
) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    headers.iter().find_map(|(k, v)| {
        if k.to_ascii_lowercase() == lower {
            match v {
                serde_json::Value::String(s) => Some(s.clone()),
                serde_json::Value::Array(arr) => arr
                    .iter()
                    .find_map(|val| val.as_str().map(|s| s.to_string())),
                _ => None,
            }
        } else {
            None
        }
    })
}

#[derive(Debug)]
enum VerificationError {
    MissingTimestamp,
    MissingSignature,
    InvalidKey,
    SignatureMismatch,
}

impl std::fmt::Display for VerificationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VerificationError::MissingTimestamp => write!(f, "missing timestamp"),
            VerificationError::MissingSignature => write!(f, "missing signature"),
            VerificationError::InvalidKey => write!(f, "invalid signing secret"),
            VerificationError::SignatureMismatch => write!(f, "signature mismatch"),
        }
    }
}

impl std::error::Error for VerificationError {}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut out = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        out |= x ^ y;
    }
    out == 0
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
        provider: "slack".into(),
        start_ms: None,
        end_ms: None,
    };
    #[cfg(not(test))]
    {
        let fields = [("event".to_string(), event.to_string())];
        let _ = logger_api::log(&span, &fields, None);
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
type HttpSendMock = dyn Fn(
    &client::Request,
    &client::RequestOptions,
) -> Result<client::Response, client::HostError>;

#[cfg(test)]
thread_local! {
    static HTTP_SEND_MOCK: std::cell::RefCell<Option<Box<HttpSendMock>>> =
        std::cell::RefCell::new(None);
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
            "api_base_url": "https://slack.com/api",
            "bot_token": "x",
            "unknown": true
        });
        let err = load_config(&value).unwrap_err();
        assert!(err.contains("unknown field"));
    }

    #[test]
    fn parse_config_requires_new_fields() {
        let value = json!({"enabled": true, "api_base_url": "https://slack.com/api"});
        let err = load_config(&value).unwrap_err();
        assert!(err.contains("bot_token") || err.contains("public_base_url"));
    }

    #[test]
    fn invoke_run_requires_message() {
        let input = json!({
            "config": {
                "enabled": true,
                "public_base_url": "https://example.com",
                "api_base_url": "https://slack.com/api",
                "bot_token": "token",
                "default_channel": "C123"
            }
        });
        let out = handle_send(&input);
        assert_eq!(out["ok"], Value::Bool(false));
    }

    #[test]
    fn send_uses_http_mock() {
        let input = json!({
            "message": "hello",
            "channel": "C123",
            "config": {
                "enabled": true,
                "public_base_url": "https://example.com",
                "api_base_url": "https://slack.com/api",
                "bot_token": "token"
            }
        });

        with_http_send_mock(
            |req, _| {
                assert!(req.url.ends_with("/chat.postMessage"));
                Ok(client::Response {
                    status: 200,
                    headers: vec![],
                    body: Some(br#"{"ok":true,"ts":"123.456"}"#.to_vec()),
                })
            },
            || {
                let out = handle_send(&input);
                assert_eq!(out["ok"], Value::Bool(true));
                assert_eq!(out["message_id"], Value::String("123.456".to_string()));
            },
        );
    }

    #[test]
    fn schema_hash_is_stable() {
        let describe = build_describe_payload();
        assert_eq!(
            describe.schema_hash,
            "e97582985f10cef61b11d04dbfa16e8cb6dfc5a16b786e291299850ec6197bce"
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
    fn verifies_signature() {
        let secret = "test-secret";
        let ts = "1531420618";
        let body = "token=OneLongToken&team_id=T1&api_app_id=A1&event=hello";
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(format!("v0:{}:{}", ts, body).as_bytes());
        let computed = mac.finalize().into_bytes();
        let mut hex = String::new();
        for byte in computed {
            use std::fmt::Write;
            write!(&mut hex, "{:02x}", byte).unwrap();
        }
        let sig = format!("v0={}", hex);

        let mut headers = serde_json::Map::new();
        headers.insert(
            "X-Slack-Request-Timestamp".into(),
            serde_json::Value::String(ts.to_string()),
        );
        headers.insert("X-Slack-Signature".into(), serde_json::Value::String(sig));

        verify_signature(&headers, body, secret).expect("signature should verify");
    }
}
