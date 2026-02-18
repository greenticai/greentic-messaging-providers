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
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use urlencoding::encode;

#[allow(clippy::too_many_arguments)]
mod bindings {
    wit_bindgen::generate!({ path: "wit/teams", world: "component-v0-v6-v0", generate_all });
}

const PROVIDER_ID: &str = "teams";
const WORLD_ID: &str = "component-v0-v6-v0";
const DEFAULT_GRAPH_BASE: &str = "https://graph.microsoft.com/v1.0";
const DEFAULT_AUTH_BASE: &str = "https://login.microsoftonline.com";
const DEFAULT_TOKEN_SCOPE: &str = "https://graph.microsoft.com/.default";
const SECRET_CLIENT_SECRET: &str = "MS_GRAPH_CLIENT_SECRET";

const I18N_KEYS: &[&str] = &[
    "teams.op.run.title",
    "teams.op.run.description",
    "teams.op.send.title",
    "teams.op.send.description",
    "teams.op.ingest_http.title",
    "teams.op.ingest_http.description",
    "teams.op.encode.title",
    "teams.op.encode.description",
    "teams.op.send_payload.title",
    "teams.op.send_payload.description",
    "teams.schema.input.title",
    "teams.schema.input.description",
    "teams.schema.input.message.title",
    "teams.schema.input.message.description",
    "teams.schema.output.title",
    "teams.schema.output.description",
    "teams.schema.output.ok.title",
    "teams.schema.output.ok.description",
    "teams.schema.output.message_id.title",
    "teams.schema.output.message_id.description",
    "teams.schema.config.title",
    "teams.schema.config.description",
    "teams.schema.config.enabled.title",
    "teams.schema.config.enabled.description",
    "teams.schema.config.tenant_id.title",
    "teams.schema.config.tenant_id.description",
    "teams.schema.config.client_id.title",
    "teams.schema.config.client_id.description",
    "teams.schema.config.public_base_url.title",
    "teams.schema.config.public_base_url.description",
    "teams.schema.config.graph_base_url.title",
    "teams.schema.config.graph_base_url.description",
    "teams.schema.config.auth_base_url.title",
    "teams.schema.config.auth_base_url.description",
    "teams.schema.config.token_scope.title",
    "teams.schema.config.token_scope.description",
    "teams.schema.config.team_id.title",
    "teams.schema.config.team_id.description",
    "teams.schema.config.channel_id.title",
    "teams.schema.config.channel_id.description",
    "teams.schema.config.client_secret.title",
    "teams.schema.config.client_secret.description",
    "teams.schema.config.refresh_token.title",
    "teams.schema.config.refresh_token.description",
    "teams.qa.default.title",
    "teams.qa.setup.title",
    "teams.qa.upgrade.title",
    "teams.qa.remove.title",
    "teams.qa.setup.enabled",
    "teams.qa.setup.tenant_id",
    "teams.qa.setup.client_id",
    "teams.qa.setup.public_base_url",
    "teams.qa.setup.graph_base_url",
    "teams.qa.setup.auth_base_url",
    "teams.qa.setup.token_scope",
    "teams.qa.setup.team_id",
    "teams.qa.setup.channel_id",
    "teams.qa.setup.client_secret",
];

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProviderConfig {
    #[serde(default = "default_enabled")]
    enabled: bool,
    tenant_id: String,
    client_id: String,
    public_base_url: String,
    #[serde(default = "default_graph_base")]
    graph_base_url: String,
    #[serde(default = "default_auth_base")]
    auth_base_url: String,
    #[serde(default = "default_token_scope")]
    token_scope: String,
    #[serde(default)]
    team_id: Option<String>,
    #[serde(default)]
    channel_id: Option<String>,
    #[serde(default)]
    client_secret: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ApplyAnswersResult {
    ok: bool,
    config: Option<ProviderConfig>,
    error: Option<String>,
}

#[derive(Debug)]
struct Destination {
    team_id: String,
    channel_id: String,
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
                tenant_id: answers
                    .get("tenant_id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string(),
                client_id: answers
                    .get("client_id")
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
                graph_base_url: answers
                    .get("graph_base_url")
                    .and_then(Value::as_str)
                    .unwrap_or(DEFAULT_GRAPH_BASE)
                    .trim()
                    .to_string(),
                auth_base_url: answers
                    .get("auth_base_url")
                    .and_then(Value::as_str)
                    .unwrap_or(DEFAULT_AUTH_BASE)
                    .trim()
                    .to_string(),
                token_scope: answers
                    .get("token_scope")
                    .and_then(Value::as_str)
                    .unwrap_or(DEFAULT_TOKEN_SCOPE)
                    .trim()
                    .to_string(),
                team_id: answers
                    .get("team_id")
                    .and_then(Value::as_str)
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
                channel_id: answers
                    .get("channel_id")
                    .and_then(Value::as_str)
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
                client_secret: answers
                    .get("client_secret")
                    .and_then(Value::as_str)
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
                refresh_token: answers
                    .get("refresh_token")
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
            op("run", "teams.op.run.title", "teams.op.run.description"),
            op("send", "teams.op.send.title", "teams.op.send.description"),
            op(
                "ingest_http",
                "teams.op.ingest_http.title",
                "teams.op.ingest_http.description",
            ),
            op(
                "encode",
                "teams.op.encode.title",
                "teams.op.encode.description",
            ),
            op(
                "send_payload",
                "teams.op.send_payload.title",
                "teams.op.send_payload.description",
            ),
        ],
        input_schema: input_schema.clone(),
        output_schema: output_schema.clone(),
        config_schema: config_schema.clone(),
        redactions: vec![
            RedactionRule {
                path: "$.client_secret".to_string(),
                strategy: "replace".to_string(),
            },
            RedactionRule {
                path: "$.refresh_token".to_string(),
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
            title: i18n("teams.qa.default.title"),
            questions: Vec::new(),
        },
        Mode::Setup => QaSpec {
            mode: "setup".to_string(),
            title: i18n("teams.qa.setup.title"),
            questions: vec![
                qa_q("enabled", "teams.qa.setup.enabled", true),
                qa_q("tenant_id", "teams.qa.setup.tenant_id", true),
                qa_q("client_id", "teams.qa.setup.client_id", true),
                qa_q("public_base_url", "teams.qa.setup.public_base_url", true),
                qa_q("graph_base_url", "teams.qa.setup.graph_base_url", true),
                qa_q("auth_base_url", "teams.qa.setup.auth_base_url", true),
                qa_q("token_scope", "teams.qa.setup.token_scope", true),
                qa_q("team_id", "teams.qa.setup.team_id", false),
                qa_q("channel_id", "teams.qa.setup.channel_id", false),
                qa_q("client_secret", "teams.qa.setup.client_secret", false),
            ],
        },
        Mode::Upgrade => QaSpec {
            mode: "upgrade".to_string(),
            title: i18n("teams.qa.upgrade.title"),
            questions: Vec::new(),
        },
        Mode::Remove => QaSpec {
            mode: "remove".to_string(),
            title: i18n("teams.qa.remove.title"),
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
                title: i18n("teams.schema.input.message.title"),
                description: i18n("teams.schema.input.message.description"),
                format: None,
                secret: false,
            },
        },
    );

    SchemaIr::Object {
        title: i18n("teams.schema.input.title"),
        description: i18n("teams.schema.input.description"),
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
                title: i18n("teams.schema.output.ok.title"),
                description: i18n("teams.schema.output.ok.description"),
            },
        },
    );
    fields.insert(
        "message_id".to_string(),
        SchemaField {
            required: false,
            schema: SchemaIr::String {
                title: i18n("teams.schema.output.message_id.title"),
                description: i18n("teams.schema.output.message_id.description"),
                format: None,
                secret: false,
            },
        },
    );

    SchemaIr::Object {
        title: i18n("teams.schema.output.title"),
        description: i18n("teams.schema.output.description"),
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
                title: i18n("teams.schema.config.enabled.title"),
                description: i18n("teams.schema.config.enabled.description"),
            },
        },
    );
    fields.insert(
        "tenant_id".to_string(),
        SchemaField {
            required: true,
            schema: SchemaIr::String {
                title: i18n("teams.schema.config.tenant_id.title"),
                description: i18n("teams.schema.config.tenant_id.description"),
                format: None,
                secret: false,
            },
        },
    );
    fields.insert(
        "client_id".to_string(),
        SchemaField {
            required: true,
            schema: SchemaIr::String {
                title: i18n("teams.schema.config.client_id.title"),
                description: i18n("teams.schema.config.client_id.description"),
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
                title: i18n("teams.schema.config.public_base_url.title"),
                description: i18n("teams.schema.config.public_base_url.description"),
                format: Some("uri".to_string()),
                secret: false,
            },
        },
    );
    fields.insert(
        "graph_base_url".to_string(),
        SchemaField {
            required: true,
            schema: SchemaIr::String {
                title: i18n("teams.schema.config.graph_base_url.title"),
                description: i18n("teams.schema.config.graph_base_url.description"),
                format: Some("uri".to_string()),
                secret: false,
            },
        },
    );
    fields.insert(
        "auth_base_url".to_string(),
        SchemaField {
            required: true,
            schema: SchemaIr::String {
                title: i18n("teams.schema.config.auth_base_url.title"),
                description: i18n("teams.schema.config.auth_base_url.description"),
                format: Some("uri".to_string()),
                secret: false,
            },
        },
    );
    fields.insert(
        "token_scope".to_string(),
        SchemaField {
            required: true,
            schema: SchemaIr::String {
                title: i18n("teams.schema.config.token_scope.title"),
                description: i18n("teams.schema.config.token_scope.description"),
                format: None,
                secret: false,
            },
        },
    );
    fields.insert(
        "team_id".to_string(),
        SchemaField {
            required: false,
            schema: SchemaIr::String {
                title: i18n("teams.schema.config.team_id.title"),
                description: i18n("teams.schema.config.team_id.description"),
                format: None,
                secret: false,
            },
        },
    );
    fields.insert(
        "channel_id".to_string(),
        SchemaField {
            required: false,
            schema: SchemaIr::String {
                title: i18n("teams.schema.config.channel_id.title"),
                description: i18n("teams.schema.config.channel_id.description"),
                format: None,
                secret: false,
            },
        },
    );
    fields.insert(
        "client_secret".to_string(),
        SchemaField {
            required: false,
            schema: SchemaIr::String {
                title: i18n("teams.schema.config.client_secret.title"),
                description: i18n("teams.schema.config.client_secret.description"),
                format: None,
                secret: true,
            },
        },
    );
    fields.insert(
        "refresh_token".to_string(),
        SchemaField {
            required: false,
            schema: SchemaIr::String {
                title: i18n("teams.schema.config.refresh_token.title"),
                description: i18n("teams.schema.config.refresh_token.description"),
                format: None,
                secret: true,
            },
        },
    );

    SchemaIr::Object {
        title: i18n("teams.schema.config.title"),
        description: i18n("teams.schema.config.description"),
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

    let message = input
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| input.get("text").and_then(Value::as_str))
        .map(str::trim)
        .unwrap_or("");
    if message.is_empty() {
        return json!({"ok": false, "error": "missing message"});
    }

    let destination = match resolve_destination(input, &cfg) {
        Ok(dest) => dest,
        Err(err) => return json!({"ok": false, "error": err}),
    };

    let access_token = match get_access_token(&cfg) {
        Ok(token) => token,
        Err(err) => return json!({"ok": false, "error": err}),
    };

    let payload = json!({"body": {"contentType": "html", "content": message}});

    if input
        .get("dry_run")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return json!({
            "ok": true,
            "message_id": payload_hash(&payload),
            "dry_run": true,
            "request": {
                "team_id": destination.team_id,
                "channel_id": destination.channel_id,
                "payload": payload,
            }
        });
    }

    let url = format!(
        "{}/teams/{}/channels/{}/messages",
        cfg.graph_base_url.trim_end_matches('/'),
        destination.team_id,
        destination.channel_id
    );
    let req = client::Request {
        method: "POST".into(),
        url,
        headers: vec![
            ("Content-Type".into(), "application/json".into()),
            ("Authorization".into(), format!("Bearer {}", access_token)),
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
            let message_id = resp
                .body
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .and_then(|v| {
                    v.get("id")
                        .and_then(Value::as_str)
                        .map(ToString::to_string)
                        .or_else(|| {
                            v.get("messageId")
                                .and_then(Value::as_str)
                                .map(ToString::to_string)
                        })
                })
                .unwrap_or_else(|| payload_hash(&payload));
            json!({"ok": true, "message_id": message_id, "status": resp.status})
        }
        Ok(resp) => {
            json!({"ok": false, "error": format!("transport error: graph returned status {}", resp.status)})
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
    let payload = json!({"body": {"contentType": "html", "content": text}});
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

fn resolve_destination(input: &Value, cfg: &ProviderConfig) -> Result<Destination, String> {
    let team_id = input
        .get("team_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .or_else(|| cfg.team_id.clone())
        .ok_or_else(|| "missing team_id and no default configured".to_string())?;

    let channel_id = input
        .get("channel_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .or_else(|| cfg.channel_id.clone())
        .ok_or_else(|| "missing channel_id and no default configured".to_string())?;

    Ok(Destination {
        team_id,
        channel_id,
    })
}

fn get_access_token(cfg: &ProviderConfig) -> Result<String, String> {
    let client_secret = if let Some(secret) = cfg.client_secret.as_ref() {
        secret.clone()
    } else {
        get_secret_string(SECRET_CLIENT_SECRET)?
    };

    let token_url = format!(
        "{}/{}/oauth2/v2.0/token",
        cfg.auth_base_url.trim_end_matches('/'),
        cfg.tenant_id,
    );
    let form = format!(
        "client_id={}&client_secret={}&grant_type=client_credentials&scope={}",
        encode(&cfg.client_id),
        encode(&client_secret),
        encode(&cfg.token_scope)
    );

    let req = client::Request {
        method: "POST".into(),
        url: token_url,
        headers: vec![(
            "Content-Type".into(),
            "application/x-www-form-urlencoded".into(),
        )],
        body: Some(form.into_bytes()),
    };
    let options = client::RequestOptions {
        timeout_ms: None,
        allow_insecure: Some(false),
        follow_redirects: None,
    };

    let resp = http_send(&req, &options)
        .map_err(|err| format!("transport error: {} ({})", err.message, err.code))?;

    if !(200..300).contains(&resp.status) {
        return Err(format!(
            "transport error: token endpoint returned status {}",
            resp.status
        ));
    }

    let value: Value = serde_json::from_slice(&resp.body.unwrap_or_default())
        .map_err(|_| "other error: invalid token response".to_string())?;

    value
        .get("access_token")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| "other error: token response missing access_token".to_string())
}

fn default_enabled() -> bool {
    true
}

fn default_graph_base() -> String {
    DEFAULT_GRAPH_BASE.to_string()
}

fn default_auth_base() -> String {
    DEFAULT_AUTH_BASE.to_string()
}

fn default_token_scope() -> String {
    DEFAULT_TOKEN_SCOPE.to_string()
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
    if cfg.tenant_id.trim().is_empty() {
        return Err("tenant_id must be non-empty".to_string());
    }
    if cfg.client_id.trim().is_empty() {
        return Err("client_id must be non-empty".to_string());
    }
    if cfg.public_base_url.trim().is_empty() {
        return Err("public_base_url must be non-empty".to_string());
    }
    if cfg.graph_base_url.trim().is_empty() {
        return Err("graph_base_url must be non-empty".to_string());
    }
    if cfg.auth_base_url.trim().is_empty() {
        return Err("auth_base_url must be non-empty".to_string());
    }
    if cfg.token_scope.trim().is_empty() {
        return Err("token_scope must be non-empty".to_string());
    }
    Ok(())
}

fn payload_hash(value: &Value) -> String {
    let mut hasher = Sha256::new();
    hasher.update(serde_json::to_vec(value).unwrap_or_default());
    format!("{:x}", hasher.finalize())
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
        provider: "teams".into(),
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
            "tenant_id": "t",
            "client_id": "c",
            "public_base_url": "https://example.com",
            "graph_base_url": "https://graph.microsoft.com/v1.0",
            "auth_base_url": "https://login.microsoftonline.com",
            "token_scope": "https://graph.microsoft.com/.default",
            "unknown": true
        });
        let err = load_config(&value).unwrap_err();
        assert!(err.contains("unknown field"));
    }

    #[test]
    fn parse_config_requires_new_fields() {
        let value = json!({"enabled": true, "tenant_id": "t"});
        let err = load_config(&value).unwrap_err();
        assert!(err.contains("client_id") || err.contains("public_base_url"));
    }

    #[test]
    fn invoke_run_requires_message() {
        let input = json!({
            "config": {
                "enabled": true,
                "tenant_id": "tenant",
                "client_id": "client",
                "public_base_url": "https://example.com",
                "graph_base_url": "https://graph.microsoft.com/v1.0",
                "auth_base_url": "https://login.microsoftonline.com",
                "token_scope": "https://graph.microsoft.com/.default",
                "team_id": "team",
                "channel_id": "chan",
                "client_secret": "secret"
            }
        });
        let out = handle_send(&input);
        assert_eq!(out["ok"], Value::Bool(false));
    }

    #[test]
    fn send_uses_http_mocks() {
        let input = json!({
            "message": "hello teams",
            "config": {
                "enabled": true,
                "tenant_id": "tenant",
                "client_id": "client",
                "public_base_url": "https://example.com",
                "graph_base_url": "https://graph.microsoft.com/v1.0",
                "auth_base_url": "https://login.microsoftonline.com",
                "token_scope": "https://graph.microsoft.com/.default",
                "team_id": "team",
                "channel_id": "chan",
                "client_secret": "secret"
            }
        });

        with_http_send_mock(
            |req, _| {
                if req.url.contains("oauth2/v2.0/token") {
                    return Ok(client::Response {
                        status: 200,
                        headers: vec![],
                        body: Some(br#"{"access_token":"abc"}"#.to_vec()),
                    });
                }
                Ok(client::Response {
                    status: 201,
                    headers: vec![],
                    body: Some(br#"{"id":"msg-123"}"#.to_vec()),
                })
            },
            || {
                let out = handle_send(&input);
                assert_eq!(out["ok"], Value::Bool(true));
                assert_eq!(out["message_id"], Value::String("msg-123".to_string()));
            },
        );
    }

    #[test]
    fn secret_fallback_used_when_client_secret_missing() {
        let cfg = ProviderConfig {
            enabled: true,
            tenant_id: "tenant".into(),
            client_id: "client".into(),
            public_base_url: "https://example.com".into(),
            graph_base_url: DEFAULT_GRAPH_BASE.into(),
            auth_base_url: DEFAULT_AUTH_BASE.into(),
            token_scope: DEFAULT_TOKEN_SCOPE.into(),
            team_id: Some("team".into()),
            channel_id: Some("chan".into()),
            client_secret: None,
            refresh_token: None,
        };

        with_secrets_get_mock(
            |name| {
                if name == SECRET_CLIENT_SECRET {
                    Ok(Some(b"secret-from-store".to_vec()))
                } else {
                    Ok(None)
                }
            },
            || {
                with_http_send_mock(
                    |_, _| {
                        Ok(client::Response {
                            status: 200,
                            headers: vec![],
                            body: Some(br#"{"access_token":"abc"}"#.to_vec()),
                        })
                    },
                    || {
                        let token = get_access_token(&cfg).expect("token");
                        assert_eq!(token, "abc");
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
            "6eeefd5235cda241a0c38d9748f6a224779e6db1b73b2cd9947ef52a23d8462d"
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
