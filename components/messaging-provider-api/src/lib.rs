//! `messaging-api`: a programmatic messaging channel (REST send; SSE, polling
//! or webhook receive). All rules live in `api-channel-core`; this crate is the
//! WASM adapter that binds them to the host capabilities.

use provider_common::component_v0_6::{
    DescribePayload, OperationDescriptor, RedactionRule, SchemaIr, canonical_cbor_bytes,
    decode_cbor, schema_hash,
};
use provider_common::helpers::i18n;
use serde_json::{Value, json};

mod bindings {
    wit_bindgen::generate!({
        path: "wit/messaging-provider-api",
        world: "component-v0-v6-v0",
        generate_all
    });
}

mod host;
mod ops;

const PROVIDER_ID: &str = "messaging-provider-api";
const WORLD_ID: &str = "component-v0-v6-v0";

const SETUP_QUESTIONS: &[provider_common::helpers::QaQuestionDef] =
    api_channel_core::setup::SETUP_QUESTIONS;
const DEFAULT_KEYS: &[&str] = &["api_key"];

const I18N_PAIRS: &[(&str, &str)] = &[
    ("api.op.send.title", "Send"),
    (
        "api.op.send.description",
        "Append a reply to the conversation",
    ),
    ("api.op.ingest_http.title", "Ingest HTTP"),
    (
        "api.op.ingest_http.description",
        "Handle POST and GET /messages",
    ),
    ("api.op.poll_events.title", "Poll events"),
    (
        "api.op.poll_events.description",
        "Read conversation events from a cursor",
    ),
    ("api.schema.input.title", "API channel input"),
    (
        "api.schema.input.description",
        "Input of the API channel operations",
    ),
    ("api.schema.output.title", "API channel output"),
    (
        "api.schema.output.description",
        "Result of the API channel operations",
    ),
    ("api.schema.config.title", "API channel config"),
    ("api.schema.config.description", "API channel configuration"),
    ("api.schema.config.webhook_url.title", "Webhook URL"),
    (
        "api.schema.config.webhook_url.description",
        "HTTPS endpoint that receives signed events",
    ),
    ("api.schema.config.enabled.title", "Enabled"),
    (
        "api.schema.config.enabled.description",
        "Enable this channel",
    ),
    ("api.qa.default.title", "Default"),
    ("api.qa.setup.title", "Setup"),
    ("api.qa.upgrade.title", "Upgrade"),
    ("api.qa.remove.title", "Remove"),
    (
        "api.qa.setup.webhook_url",
        "Webhook URL (optional, https only)",
    ),
    ("api.qa.setup.webhook_secret", "Webhook signing secret"),
    (
        "api.qa.setup.api_key",
        "API key clients present as a bearer token",
    ),
];

fn i18n_keys() -> Vec<&'static str> {
    I18N_PAIRS.iter().map(|(key, _)| *key).collect()
}

struct Component;

impl bindings::exports::greentic::component::descriptor::Guest for Component {
    fn describe() -> Vec<u8> {
        canonical_cbor_bytes(&build_describe_payload())
    }
}

impl bindings::exports::greentic::component::runtime::Guest for Component {
    fn invoke(op: String, input_cbor: Vec<u8>) -> Vec<u8> {
        // The CBOR door carries the same JSON ops; transcode both ways.
        let input: Value = decode_cbor(&input_cbor).unwrap_or(Value::Null);
        let input_json = serde_json::to_vec(&input).unwrap_or_default();
        let out = ops::dispatch(&op, &input_json);
        let value: Value = serde_json::from_slice(&out).unwrap_or(Value::Null);
        canonical_cbor_bytes(&value)
    }
}

impl bindings::exports::greentic::component::qa::Guest for Component {
    fn qa_spec(mode: bindings::exports::greentic::component::qa::Mode) -> Vec<u8> {
        canonical_cbor_bytes(&build_qa_spec(mode_str(mode)))
    }

    fn apply_answers(
        mode: bindings::exports::greentic::component::qa::Mode,
        answers_cbor: Vec<u8>,
    ) -> Vec<u8> {
        apply_answers_impl(mode_str(mode), answers_cbor)
    }
}

fn mode_str(mode: bindings::exports::greentic::component::qa::Mode) -> &'static str {
    use bindings::exports::greentic::component::qa::Mode;
    match mode {
        Mode::Default => "default",
        Mode::Setup => "setup",
        Mode::Upgrade => "upgrade",
        Mode::Remove => "remove",
    }
}

impl bindings::exports::greentic::component::component_i18n::Guest for Component {
    fn i18n_keys() -> Vec<String> {
        provider_common::helpers::i18n_keys_from(&i18n_keys())
    }

    fn i18n_bundle(locale: String) -> Vec<u8> {
        provider_common::helpers::i18n_bundle_from_pairs(locale, I18N_PAIRS)
    }
}

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
        if let Some(result) = provider_common::qa_invoke_bridge::dispatch_qa_ops_with_i18n(
            &op,
            &input_json,
            "api",
            SETUP_QUESTIONS,
            DEFAULT_KEYS,
            &i18n_keys(),
            I18N_PAIRS,
            apply_answers_impl,
        ) {
            return result;
        }
        ops::dispatch(&op, &input_json)
    }
}

impl bindings::exports::provider::common0_0_2::ingress::Guest for Component {
    /// The runtime packs `method`/`path`/`query` into the headers object and
    /// passes the raw body separately; rebuild the `HttpInV1` the op expects.
    fn handle_webhook(headers_json: String, body_json: String) -> Result<String, String> {
        handle_ingress(&headers_json, &body_json, None)
    }
}

impl bindings::exports::provider::common0_0_3::ingress::Guest for Component {
    fn handle_webhook(
        headers_json: String,
        body_json: String,
        config_json: String,
    ) -> Result<String, String> {
        let config = match serde_json::from_str::<Value>(&config_json) {
            Ok(Value::Null) | Err(_) => None,
            Ok(value) => Some(value),
        };
        handle_ingress(&headers_json, &body_json, config)
    }
}

fn handle_ingress(
    headers_json: &str,
    body_json: &str,
    config: Option<Value>,
) -> Result<String, String> {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let headers: serde_json::Map<String, Value> = serde_json::from_str(headers_json)
        .map_err(|err| format!("invalid ingress headers json: {err}"))?;
    let text = |key: &str| {
        headers
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let method = match text("method") {
        m if m.is_empty() => "POST".to_string(),
        m => m,
    };
    let pairs: Vec<Value> = headers
        .iter()
        .filter(|(key, _)| !matches!(key.as_str(), "method" | "path" | "query"))
        .map(|(key, value)| json!({"name": key, "value": value.as_str().unwrap_or_default()}))
        .collect();
    let mut input = json!({
        "method": method,
        "path": text("path"),
        "query": text("query"),
        "headers": pairs,
        "body_b64": STANDARD.encode(body_json.as_bytes()),
    });
    if let (Some(config), Some(map)) = (config, input.as_object_mut()) {
        map.insert("config".into(), config);
    }
    let bytes = serde_json::to_vec(&input).map_err(|err| format!("encode ingress: {err}"))?;
    let out = ops::dispatch("ingest_http", &bytes);
    String::from_utf8(out).map_err(|err| format!("ingress response not utf-8: {err}"))
}

fn build_qa_spec(mode: &str) -> provider_common::component_v0_6::QaSpec {
    provider_common::helpers::qa_spec_for_mode(mode, "api", SETUP_QUESTIONS, DEFAULT_KEYS)
}

fn apply_answers_impl(mode: &str, answers_cbor: Vec<u8>) -> Vec<u8> {
    let answers: Value = match decode_cbor(&answers_cbor) {
        Ok(value) => value,
        Err(err) => {
            return canonical_cbor_bytes(&json!({
                "ok": false,
                "error": format!("invalid answers cbor: {err}"),
            }));
        }
    };
    canonical_cbor_bytes(&api_channel_core::setup::apply_answers(mode, &answers))
}

fn build_describe_payload() -> DescribePayload {
    let input_schema = object_schema(
        "api.schema.input.title",
        "api.schema.input.description",
        Vec::new(),
    );
    let output_schema = object_schema(
        "api.schema.output.title",
        "api.schema.output.description",
        Vec::new(),
    );
    let config_schema = provider_common::helpers::schema_obj(
        "api.schema.config.title",
        "api.schema.config.description",
        vec![
            (
                "enabled",
                true,
                provider_common::helpers::schema_bool_ir(
                    "api.schema.config.enabled.title",
                    "api.schema.config.enabled.description",
                ),
            ),
            (
                "webhook_url",
                false,
                provider_common::helpers::schema_str_fmt(
                    "api.schema.config.webhook_url.title",
                    "api.schema.config.webhook_url.description",
                    "uri",
                ),
            ),
        ],
        false,
    );
    let hash = schema_hash(&input_schema, &output_schema, &config_schema);
    let operation = |name: &str, key: &str| OperationDescriptor {
        name: name.to_string(),
        title: i18n(&format!("api.op.{key}.title")),
        description: i18n(&format!("api.op.{key}.description")),
    };
    DescribePayload {
        provider: PROVIDER_ID.to_string(),
        world: WORLD_ID.to_string(),
        operations: vec![
            operation("send", "send"),
            operation("ingest_http", "ingest_http"),
            operation("poll_events", "poll_events"),
        ],
        input_schema,
        output_schema,
        config_schema,
        redactions: vec![
            RedactionRule {
                path: "$.api_key".to_string(),
                strategy: "replace".to_string(),
            },
            RedactionRule {
                path: "$.webhook_secret".to_string(),
                strategy: "replace".to_string(),
            },
        ],
        schema_hash: hash,
    }
}

fn object_schema(title: &str, description: &str, fields: Vec<(&str, bool, SchemaIr)>) -> SchemaIr {
    provider_common::helpers::schema_obj(title, description, fields, true)
}

bindings::export!(Component with_types_in bindings);

#[cfg(test)]
mod tests;
