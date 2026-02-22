#![allow(unsafe_op_in_unsafe_fn)]

use bindings::greentic::http::http_client as client;
#[cfg(not(test))]
use bindings::greentic::telemetry::logger_api;
use provider_common::component_v0_6::{
    DescribePayload, I18nText, OperationDescriptor, QaQuestionSpec, QaSpec, SchemaField, SchemaIr,
    canonical_cbor_bytes, decode_cbor, schema_hash,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[allow(clippy::too_many_arguments)]
mod bindings {
    wit_bindgen::generate!({ path: "wit/webchat", world: "component-v0-v6-v0", generate_all });
}

const PROVIDER_ID: &str = "webchat";
const WORLD_ID: &str = "component-v0-v6-v0";
const DEFAULT_MODE: &str = "local_queue";
const DEFAULT_SEND_URL: &str = "https://example.invalid/webchat/send";

const I18N_KEYS: &[&str] = &[
    "webchat.op.run.title",
    "webchat.op.run.description",
    "webchat.op.send.title",
    "webchat.op.send.description",
    "webchat.op.ingest_http.title",
    "webchat.op.ingest_http.description",
    "webchat.op.encode.title",
    "webchat.op.encode.description",
    "webchat.op.send_payload.title",
    "webchat.op.send_payload.description",
    "webchat.schema.input.title",
    "webchat.schema.input.description",
    "webchat.schema.input.message.title",
    "webchat.schema.input.message.description",
    "webchat.schema.output.title",
    "webchat.schema.output.description",
    "webchat.schema.output.ok.title",
    "webchat.schema.output.ok.description",
    "webchat.schema.output.message_id.title",
    "webchat.schema.output.message_id.description",
    "webchat.schema.config.title",
    "webchat.schema.config.description",
    "webchat.schema.config.enabled.title",
    "webchat.schema.config.enabled.description",
    "webchat.schema.config.public_base_url.title",
    "webchat.schema.config.public_base_url.description",
    "webchat.schema.config.mode.title",
    "webchat.schema.config.mode.description",
    "webchat.schema.config.route.title",
    "webchat.schema.config.route.description",
    "webchat.schema.config.tenant_channel_id.title",
    "webchat.schema.config.tenant_channel_id.description",
    "webchat.schema.config.base_url.title",
    "webchat.schema.config.base_url.description",
    "webchat.qa.default.title",
    "webchat.qa.setup.title",
    "webchat.qa.upgrade.title",
    "webchat.qa.remove.title",
    "webchat.qa.setup.enabled",
    "webchat.qa.setup.public_base_url",
    "webchat.qa.setup.mode",
    "webchat.qa.setup.route",
    "webchat.qa.setup.tenant_channel_id",
    "webchat.qa.setup.base_url",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderConfig {
    #[serde(default = "default_enabled")]
    enabled: bool,
    public_base_url: String,
    #[serde(default = "default_mode")]
    mode: String,
    #[serde(default)]
    route: Option<String>,
    #[serde(default)]
    tenant_channel_id: Option<String>,
    #[serde(default)]
    base_url: Option<String>,
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
                mode: answers
                    .get("mode")
                    .and_then(Value::as_str)
                    .unwrap_or(DEFAULT_MODE)
                    .trim()
                    .to_string(),
                route: answers
                    .get("route")
                    .and_then(Value::as_str)
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
                tenant_channel_id: answers
                    .get("tenant_channel_id")
                    .and_then(Value::as_str)
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
                base_url: answers
                    .get("base_url")
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
            op("run", "webchat.op.run.title", "webchat.op.run.description"),
            op(
                "send",
                "webchat.op.send.title",
                "webchat.op.send.description",
            ),
            op(
                "ingest_http",
                "webchat.op.ingest_http.title",
                "webchat.op.ingest_http.description",
            ),
            op(
                "encode",
                "webchat.op.encode.title",
                "webchat.op.encode.description",
            ),
            op(
                "send_payload",
                "webchat.op.send_payload.title",
                "webchat.op.send_payload.description",
            ),
        ],
        input_schema: input_schema.clone(),
        output_schema: output_schema.clone(),
        config_schema: config_schema.clone(),
        redactions: vec![],
        schema_hash: schema_hash(&input_schema, &output_schema, &config_schema),
    }
}

fn build_qa_spec(mode: bindings::exports::greentic::component::qa::Mode) -> QaSpec {
    use bindings::exports::greentic::component::qa::Mode;

    match mode {
        Mode::Default => QaSpec {
            mode: "default".to_string(),
            title: i18n("webchat.qa.default.title"),
            description: None,
            questions: Vec::new(),
            defaults: Default::default(),
        },
        Mode::Setup => QaSpec {
            mode: "setup".to_string(),
            title: i18n("webchat.qa.setup.title"),
            description: None,
            questions: vec![
                qa_q("enabled", "webchat.qa.setup.enabled", true),
                qa_q("public_base_url", "webchat.qa.setup.public_base_url", true),
                qa_q("mode", "webchat.qa.setup.mode", true),
                qa_q("route", "webchat.qa.setup.route", false),
                qa_q(
                    "tenant_channel_id",
                    "webchat.qa.setup.tenant_channel_id",
                    false,
                ),
                qa_q("base_url", "webchat.qa.setup.base_url", false),
            ],
            defaults: Default::default(),
        },
        Mode::Upgrade => QaSpec {
            mode: "upgrade".to_string(),
            title: i18n("webchat.qa.upgrade.title"),
            description: None,
            questions: Vec::new(),
            defaults: Default::default(),
        },
        Mode::Remove => QaSpec {
            mode: "remove".to_string(),
            title: i18n("webchat.qa.remove.title"),
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
                title: i18n("webchat.schema.input.message.title"),
                description: i18n("webchat.schema.input.message.description"),
                format: None,
                secret: false,
            },
        },
    );

    SchemaIr::Object {
        title: i18n("webchat.schema.input.title"),
        description: i18n("webchat.schema.input.description"),
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
                title: i18n("webchat.schema.output.ok.title"),
                description: i18n("webchat.schema.output.ok.description"),
            },
        },
    );
    fields.insert(
        "message_id".to_string(),
        SchemaField {
            required: false,
            schema: SchemaIr::String {
                title: i18n("webchat.schema.output.message_id.title"),
                description: i18n("webchat.schema.output.message_id.description"),
                format: None,
                secret: false,
            },
        },
    );

    SchemaIr::Object {
        title: i18n("webchat.schema.output.title"),
        description: i18n("webchat.schema.output.description"),
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
                title: i18n("webchat.schema.config.enabled.title"),
                description: i18n("webchat.schema.config.enabled.description"),
            },
        },
    );
    fields.insert(
        "public_base_url".to_string(),
        SchemaField {
            required: true,
            schema: SchemaIr::String {
                title: i18n("webchat.schema.config.public_base_url.title"),
                description: i18n("webchat.schema.config.public_base_url.description"),
                format: Some("uri".to_string()),
                secret: false,
            },
        },
    );
    fields.insert(
        "mode".to_string(),
        SchemaField {
            required: true,
            schema: SchemaIr::String {
                title: i18n("webchat.schema.config.mode.title"),
                description: i18n("webchat.schema.config.mode.description"),
                format: None,
                secret: false,
            },
        },
    );
    fields.insert(
        "route".to_string(),
        SchemaField {
            required: false,
            schema: SchemaIr::String {
                title: i18n("webchat.schema.config.route.title"),
                description: i18n("webchat.schema.config.route.description"),
                format: None,
                secret: false,
            },
        },
    );
    fields.insert(
        "tenant_channel_id".to_string(),
        SchemaField {
            required: false,
            schema: SchemaIr::String {
                title: i18n("webchat.schema.config.tenant_channel_id.title"),
                description: i18n("webchat.schema.config.tenant_channel_id.description"),
                format: None,
                secret: false,
            },
        },
    );
    fields.insert(
        "base_url".to_string(),
        SchemaField {
            required: false,
            schema: SchemaIr::String {
                title: i18n("webchat.schema.config.base_url.title"),
                description: i18n("webchat.schema.config.base_url.description"),
                format: Some("uri".to_string()),
                secret: false,
            },
        },
    );

    SchemaIr::Object {
        title: i18n("webchat.schema.config.title"),
        description: i18n("webchat.schema.config.description"),
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

    let session_id = input
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .or_else(|| cfg.route.clone())
        .or_else(|| cfg.tenant_channel_id.clone())
        .unwrap_or_else(|| "default-session".to_string());

    let payload = json!({
        "session_id": session_id,
        "text": message,
        "mode": cfg.mode,
        "route": cfg.route,
        "tenant_channel_id": cfg.tenant_channel_id,
        "public_base_url": cfg.public_base_url,
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
        url: cfg
            .base_url
            .clone()
            .unwrap_or_else(|| DEFAULT_SEND_URL.to_string()),
        headers: vec![("Content-Type".into(), "application/json".into())],
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
            json!({"ok": true, "message_id": message_id(&payload), "status": resp.status})
        }
        Ok(resp) => {
            json!({"ok": false, "error": format!("transport error: webchat returned status {}", resp.status)})
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
    let message = input
        .get("summary_text")
        .and_then(Value::as_str)
        .or_else(|| input.get("message").and_then(Value::as_str))
        .unwrap_or_default();
    let payload = json!({
        "session_id": input.get("session_id").and_then(Value::as_str).unwrap_or_default(),
        "text": message,
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

fn default_enabled() -> bool {
    true
}

fn default_mode() -> String {
    DEFAULT_MODE.to_string()
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
    if cfg.mode != "local_queue" && cfg.mode != "websocket" && cfg.mode != "pubsub" {
        return Err("mode must be one of: local_queue, websocket, pubsub".to_string());
    }
    if cfg.route.as_deref().unwrap_or("").trim().is_empty()
        && cfg
            .tenant_channel_id
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
    {
        return Err("either route or tenant_channel_id must be configured".to_string());
    }
    Ok(())
}

fn message_id(payload: &Value) -> String {
    let bytes = serde_json::to_vec(payload).unwrap_or_default();
    provider_common::component_v0_6::sha256_hex(&bytes)
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
        provider: "webchat".into(),
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
            "mode": "local_queue",
            "route": "route-1",
            "unknown": true
        });
        let err = load_config(&value).unwrap_err();
        assert!(err.contains("unknown field"));
    }

    #[test]
    fn parse_config_requires_new_fields() {
        let value = json!({"enabled": true, "mode": "local_queue"});
        let err = load_config(&value).unwrap_err();
        assert!(err.contains("public_base_url"));
    }

    #[test]
    fn invoke_run_requires_message() {
        let input = json!({
            "config": {
                "enabled": true,
                "public_base_url": "https://example.com",
                "mode": "local_queue",
                "route": "r"
            }
        });
        let out = handle_send(&input);
        assert_eq!(out["ok"], Value::Bool(false));
    }

    #[test]
    fn send_uses_http_mock() {
        let input = json!({
            "message": "hello webchat",
            "session_id": "s-1",
            "config": {
                "enabled": true,
                "public_base_url": "https://example.com",
                "mode": "local_queue",
                "route": "r"
            }
        });

        with_http_send_mock(
            |_, _| {
                Ok(client::Response {
                    status: 202,
                    headers: vec![],
                    body: None,
                })
            },
            || {
                let out = handle_send(&input);
                assert_eq!(out["ok"], Value::Bool(true));
            },
        );
    }

    #[test]
    fn schema_hash_is_stable() {
        let describe = build_describe_payload();
        assert_eq!(
            describe.schema_hash,
            "19cb56f3932284b00dc8938756534ff1deb7d58ebee08a7aeed3b8abf2e53a88"
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
