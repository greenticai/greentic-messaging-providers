use base64::{Engine as _, engine::general_purpose};
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
use std::collections::{BTreeMap, HashMap};

mod bindings {
    wit_bindgen::generate!({
        path: "wit/messaging-provider-whatsapp",
        world: "component-v0-v6-v0",
        generate_all
    });
}

use bindings::greentic::http::http_client as client;
use bindings::greentic::secrets_store::secrets_store;

const PROVIDER_ID: &str = "messaging-provider-whatsapp";
const PROVIDER_TYPE: &str = "messaging.whatsapp.cloud";
const WORLD_ID: &str = "component-v0-v6-v0";
const DEFAULT_API_BASE: &str = "https://graph.facebook.com";
const DEFAULT_API_VERSION: &str = "v19.0";
const DEFAULT_TOKEN_KEY: &str = "WHATSAPP_TOKEN";
const I18N_KEYS: &[&str] = &[
    "whatsapp.op.run.title",
    "whatsapp.op.run.description",
    "whatsapp.op.send.title",
    "whatsapp.op.send.description",
    "whatsapp.op.reply.title",
    "whatsapp.op.reply.description",
    "whatsapp.op.ingest_http.title",
    "whatsapp.op.ingest_http.description",
    "whatsapp.op.render_plan.title",
    "whatsapp.op.render_plan.description",
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderConfig {
    #[serde(default = "default_enabled")]
    enabled: bool,
    phone_number_id: String,
    public_base_url: String,
    #[serde(default)]
    business_account_id: Option<String>,
    #[serde(default)]
    api_base_url: Option<String>,
    #[serde(default)]
    api_version: Option<String>,
    #[serde(default)]
    token: Option<String>,
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
    phone_number_id: String,
    public_base_url: String,
    business_account_id: Option<String>,
    api_base_url: String,
    api_version: String,
    token: Option<String>,
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
            merged.phone_number_id =
                string_or_default(&answers, "phone_number_id", &merged.phone_number_id);
            merged.public_base_url =
                string_or_default(&answers, "public_base_url", &merged.public_base_url);
            merged.business_account_id = optional_string_from(&answers, "business_account_id")
                .or(merged.business_account_id.clone());
            merged.api_base_url = string_or_default(&answers, "api_base_url", &merged.api_base_url);
            if merged.api_base_url.trim().is_empty() {
                merged.api_base_url = DEFAULT_API_BASE.to_string();
            }
            merged.api_version = string_or_default(&answers, "api_version", &merged.api_version);
            if merged.api_version.trim().is_empty() {
                merged.api_version = DEFAULT_API_VERSION.to_string();
            }
            merged.token = optional_string_from(&answers, "token").or(merged.token.clone());
        }

        if mode == bindings::exports::greentic::component::qa::Mode::Upgrade {
            if has("enabled") {
                merged.enabled = answers
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(merged.enabled);
            }
            if has("phone_number_id") {
                merged.phone_number_id =
                    string_or_default(&answers, "phone_number_id", &merged.phone_number_id);
            }
            if has("public_base_url") {
                merged.public_base_url =
                    string_or_default(&answers, "public_base_url", &merged.public_base_url);
            }
            if has("business_account_id") {
                merged.business_account_id = optional_string_from(&answers, "business_account_id");
            }
            if has("api_base_url") {
                merged.api_base_url =
                    string_or_default(&answers, "api_base_url", &merged.api_base_url);
            }
            if has("api_version") {
                merged.api_version =
                    string_or_default(&answers, "api_version", &merged.api_version);
            }
            if has("token") {
                merged.token = optional_string_from(&answers, "token");
            }
            if merged.api_base_url.trim().is_empty() {
                merged.api_base_url = DEFAULT_API_BASE.to_string();
            }
            if merged.api_version.trim().is_empty() {
                merged.api_version = DEFAULT_API_VERSION.to_string();
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
                "whatsapp.op.run.title",
                "whatsapp.op.run.description",
            ),
            op(
                "send",
                "whatsapp.op.send.title",
                "whatsapp.op.send.description",
            ),
            op(
                "reply",
                "whatsapp.op.reply.title",
                "whatsapp.op.reply.description",
            ),
            op(
                "ingest_http",
                "whatsapp.op.ingest_http.title",
                "whatsapp.op.ingest_http.description",
            ),
            op(
                "render_plan",
                "whatsapp.op.render_plan.title",
                "whatsapp.op.render_plan.description",
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
            questions: vec![
                qa_q("phone_number_id", "whatsapp.qa.setup.phone_number_id", true),
                qa_q("public_base_url", "whatsapp.qa.setup.public_base_url", true),
            ],
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
            questions: vec![
                qa_q("enabled", "whatsapp.qa.setup.enabled", false),
                qa_q(
                    "phone_number_id",
                    "whatsapp.qa.setup.phone_number_id",
                    false,
                ),
                qa_q(
                    "public_base_url",
                    "whatsapp.qa.setup.public_base_url",
                    false,
                ),
                qa_q(
                    "business_account_id",
                    "whatsapp.qa.setup.business_account_id",
                    false,
                ),
                qa_q("api_base_url", "whatsapp.qa.setup.api_base_url", false),
                qa_q("api_version", "whatsapp.qa.setup.api_version", false),
                qa_q("token", "whatsapp.qa.setup.token", false),
            ],
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
    let mut insert = |k: &str, required: bool, schema: SchemaIr| {
        fields.insert(k.to_string(), SchemaField { required, schema });
    };
    insert(
        "enabled",
        true,
        SchemaIr::Bool {
            title: i18n("whatsapp.schema.config.enabled.title"),
            description: i18n("whatsapp.schema.config.enabled.description"),
        },
    );
    insert(
        "phone_number_id",
        true,
        SchemaIr::String {
            title: i18n("whatsapp.schema.config.phone_number_id.title"),
            description: i18n("whatsapp.schema.config.phone_number_id.description"),
            format: None,
            secret: false,
        },
    );
    insert(
        "public_base_url",
        true,
        SchemaIr::String {
            title: i18n("whatsapp.schema.config.public_base_url.title"),
            description: i18n("whatsapp.schema.config.public_base_url.description"),
            format: Some("uri".to_string()),
            secret: false,
        },
    );
    insert(
        "business_account_id",
        false,
        SchemaIr::String {
            title: i18n("whatsapp.schema.config.business_account_id.title"),
            description: i18n("whatsapp.schema.config.business_account_id.description"),
            format: None,
            secret: false,
        },
    );
    insert(
        "api_base_url",
        true,
        SchemaIr::String {
            title: i18n("whatsapp.schema.config.api_base_url.title"),
            description: i18n("whatsapp.schema.config.api_base_url.description"),
            format: Some("uri".to_string()),
            secret: false,
        },
    );
    insert(
        "api_version",
        true,
        SchemaIr::String {
            title: i18n("whatsapp.schema.config.api_version.title"),
            description: i18n("whatsapp.schema.config.api_version.description"),
            format: None,
            secret: false,
        },
    );
    insert(
        "token",
        false,
        SchemaIr::String {
            title: i18n("whatsapp.schema.config.token.title"),
            description: i18n("whatsapp.schema.config.token.description"),
            format: None,
            secret: true,
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

fn handle_send(input_json: &[u8]) -> Vec<u8> {
    let parsed: Value = match serde_json::from_slice(input_json) {
        Ok(val) => val,
        Err(err) => {
            return json_bytes(&json!({"ok": false, "error": format!("invalid json: {err}")}));
        }
    };

    if let Some(rich) = parsed.get("rich")
        && rich.get("format").and_then(Value::as_str) == Some("whatsapp_template")
    {
        return json_bytes(&json!({"ok": false, "error": "template messages not supported yet"}));
    }

    let cfg = match load_config(&parsed) {
        Ok(cfg) => cfg,
        Err(err) => return json_bytes(&json!({"ok": false, "error": err})),
    };
    if !cfg.enabled {
        return json_bytes(&json!({"ok": false, "error": "provider disabled by config"}));
    }

    let envelope: ChannelMessageEnvelope = match serde_json::from_slice(input_json) {
        Ok(env) => env,
        Err(err) => match build_send_envelope_from_input(&parsed) {
            Ok(env) => env,
            Err(message) => {
                return json_bytes(
                    &json!({"ok": false, "error": format!("invalid envelope: {message}: {err}")}),
                );
            }
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

    let destination = envelope.to.first().cloned();
    let destination = match destination {
        Some(dest) => dest,
        None => return json_bytes(&json!({"ok": false, "error": "destination required"})),
    };

    let dest_id = destination.id.trim();
    if dest_id.is_empty() {
        return json_bytes(&json!({"ok": false, "error": "destination id required"}));
    }
    let kind = destination.kind.as_deref().unwrap_or("phone");
    if kind != "phone" {
        return json_bytes(&json!({
            "ok": false,
            "error": format!("unsupported destination kind: {kind}"),
        }));
    }

    let token = match get_token(&cfg) {
        Ok(token) => token,
        Err(err) => return json_bytes(&json!({"ok": false, "error": err})),
    };

    let api_base = cfg
        .api_base_url
        .clone()
        .unwrap_or_else(|| DEFAULT_API_BASE.to_string());
    let api_version = cfg
        .api_version
        .clone()
        .unwrap_or_else(|| DEFAULT_API_VERSION.to_string());
    let url = format!(
        "{}/{}/{}/messages",
        api_base, api_version, cfg.phone_number_id
    );

    let payload = json!({
        "messaging_product": "whatsapp",
        "to": dest_id,
        "type": "text",
        "text": {"body": text},
    });

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
            &json!({"ok": false, "error": format!("whatsapp returned status {}", resp.status)}),
        );
    }

    let body = resp.body.unwrap_or_default();
    let body_json: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let msg_id = body_json
        .get("messages")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|v| v.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("wa-message")
        .to_string();
    let provider_message_id = format!("whatsapp:{msg_id}");

    json_bytes(&json!({
        "ok": true,
        "status": "sent",
        "provider_type": PROVIDER_TYPE,
        "public_base_url": cfg.public_base_url,
        "message_id": msg_id,
        "provider_message_id": provider_message_id,
        "response": body_json
    }))
}

fn build_send_envelope_from_input(parsed: &Value) -> Result<ChannelMessageEnvelope, String> {
    let text = parsed
        .get("text")
        .and_then(Value::as_str)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| "text required".to_string())?;
    let destination =
        parse_send_destination(parsed).ok_or_else(|| "destination required".to_string())?;
    let env = EnvId::try_from("manual").expect("manual env id");
    let tenant = TenantId::try_from("manual").expect("manual tenant id");
    let mut metadata = MessageMetadata::new();
    metadata.insert("synthetic".to_string(), "true".to_string());
    if let Some(kind) = destination.kind.as_ref() {
        metadata.insert("destination_kind".to_string(), kind.clone());
    }
    let channel = destination.id.clone();
    Ok(ChannelMessageEnvelope {
        id: format!("whatsapp-manual-{channel}"),
        tenant: TenantCtx::new(env, tenant),
        channel: channel.clone(),
        session_id: channel,
        reply_scope: None,
        from: None,
        to: vec![destination],
        correlation_id: None,
        text: Some(text),
        attachments: Vec::new(),
        metadata,
    })
}

fn parse_send_destination(parsed: &Value) -> Option<Destination> {
    let to_value = parsed.get("to")?;
    if let Some(id) = to_value.as_str() {
        let trimmed = id.trim();
        if trimmed.is_empty() {
            return None;
        }
        return Some(Destination {
            id: trimmed.to_string(),
            kind: Some("phone".to_string()),
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
        .map(|s| s.trim().to_string());
    let kind = match kind.as_deref() {
        Some("user") => Some("phone".to_string()),
        Some(kind_str) if !kind_str.is_empty() => Some(kind_str.to_string()),
        _ => Some("phone".to_string()),
    };
    id.map(|id| Destination { id, kind })
}

fn handle_reply(_input_json: &[u8]) -> Vec<u8> {
    let parsed: Value = match serde_json::from_slice(_input_json) {
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
    let to_kind = parsed
        .get("to")
        .and_then(|v| v.get("kind"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let to_id = parsed
        .get("to")
        .and_then(|v| v.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if to_kind != "user" || to_id.is_empty() {
        return json_bytes(&json!({"ok": false, "error": "to.kind=user with to.id required"}));
    }
    let text = parsed
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if text.is_empty() {
        return json_bytes(&json!({"ok": false, "error": "text required"}));
    }
    let reply_to = parsed
        .get("reply_to_id")
        .or_else(|| parsed.get("thread_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if reply_to.is_empty() {
        return json_bytes(&json!({"ok": false, "error": "reply_to_id or thread_id required"}));
    }

    let token = match get_token(&cfg) {
        Ok(token) => token,
        Err(err) => return json_bytes(&json!({"ok": false, "error": err})),
    };
    let api_base = cfg
        .api_base_url
        .clone()
        .unwrap_or_else(|| DEFAULT_API_BASE.to_string());
    let api_version = cfg
        .api_version
        .clone()
        .unwrap_or_else(|| DEFAULT_API_VERSION.to_string());
    let url = format!(
        "{}/{}/{}/messages",
        api_base, api_version, cfg.phone_number_id
    );
    let payload = json!({
        "messaging_product": "whatsapp",
        "to": to_id,
        "type": "text",
        "context": {"message_id": reply_to},
        "text": { "body": text }
    });
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
            return json_bytes(&json!({
                "ok": false,
                "error": format!("transport error: {}", err.message),
            }));
        }
    };
    if resp.status < 200 || resp.status >= 300 {
        return json_bytes(&json!({
            "ok": false,
            "error": format!("whatsapp returned status {}", resp.status),
        }));
    }
    let body_bytes = resp.body.unwrap_or_default();
    let body_json: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    let msg_id = body_json
        .get("messages")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|v| v.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("wa-reply")
        .to_string();
    let provider_message_id = format!("whatsapp:{msg_id}");

    json_bytes(&json!({
        "ok": true,
        "status": "replied",
        "provider_type": PROVIDER_TYPE,
        "public_base_url": cfg.public_base_url,
        "message_id": msg_id,
        "provider_message_id": provider_message_id,
        "response": body_json
    }))
}

fn ingest_http(input_json: &[u8]) -> Vec<u8> {
    let request = match serde_json::from_slice::<HttpInV1>(input_json) {
        Ok(req) => req,
        Err(err) => return http_out_error(400, &format!("invalid http input: {err}")),
    };
    if request.method.eq_ignore_ascii_case("GET") {
        let challenge = parse_query(&request.query)
            .and_then(|params| params.get("hub.challenge").cloned())
            .unwrap_or_default();
        let out = HttpOutV1 {
            status: 200,
            headers: Vec::new(),
            body_b64: general_purpose::STANDARD.encode(challenge.as_bytes()),
            events: Vec::new(),
        };
        return http_out_v1_bytes(&out);
    }
    let body_bytes = match general_purpose::STANDARD.decode(&request.body_b64) {
        Ok(bytes) => bytes,
        Err(err) => return http_out_error(400, &format!("invalid body encoding: {err}")),
    };
    let body_val: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    let text = body_val
        .get("text")
        .and_then(|t| t.get("body"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let from = body_val
        .get("from")
        .and_then(Value::as_str)
        .map(str::to_string);
    let envelope = build_whatsapp_envelope(text.clone(), from.clone());
    let normalized = json!({
        "ok": true,
        "event": body_val,
        "text": text,
        "from": from,
    });
    let normalized_bytes = serde_json::to_vec(&normalized).unwrap_or_else(|_| b"{}".to_vec());
    let out = HttpOutV1 {
        status: 200,
        headers: Vec::new(),
        body_b64: general_purpose::STANDARD.encode(&normalized_bytes),
        events: vec![envelope],
    };
    http_out_v1_bytes(&out)
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
                .filter(|text| !text.trim().is_empty())
        })
        .unwrap_or_else(|| "whatsapp message".to_string());
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
    let text = encode_in
        .message
        .text
        .clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| "universal whatsapp payload".to_string());
    let to_id = encode_in
        .message
        .metadata
        .get("from")
        .cloned()
        .unwrap_or_else(|| "whatsapp-user".to_string());
    let phone_number_id = encode_in
        .message
        .metadata
        .get("phone_number_id")
        .cloned()
        .unwrap_or_else(|| "phone-universal".to_string());
    let to = json!({
        "kind": "user",
        "id": to_id,
    });
    let config = json!({
        "phone_number_id": phone_number_id,
    });
    let payload_body = json!({
        "text": text,
        "to": to,
        "config": config,
    });
    let body_bytes = serde_json::to_vec(&payload_body).unwrap_or_else(|_| b"{}".to_vec());
    let mut metadata = BTreeMap::new();
    metadata.insert("method".to_string(), Value::String("POST".to_string()));
    let payload = ProviderPayloadV1 {
        content_type: "application/json".to_string(),
        body_b64: general_purpose::STANDARD.encode(&body_bytes),
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
    let payload_bytes = match general_purpose::STANDARD.decode(&send_in.payload.body_b64) {
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

fn build_whatsapp_envelope(text: String, from: Option<String>) -> ChannelMessageEnvelope {
    let env = EnvId::try_from("default").expect("env id");
    let tenant = TenantId::try_from("default").expect("tenant id");
    let mut metadata = MessageMetadata::new();
    metadata.insert("universal".to_string(), "true".to_string());
    metadata.insert("channel_id".to_string(), "whatsapp".to_string());
    let sender = from.map(|id| Actor {
        id,
        kind: Some("user".into()),
    });
    if let Some(actor) = &sender {
        metadata.insert("from".to_string(), actor.id.clone());
    }
    ChannelMessageEnvelope {
        id: format!("whatsapp-{}", text),
        tenant: TenantCtx::new(env.clone(), tenant.clone()),
        channel: "whatsapp".to_string(),
        session_id: "whatsapp".to_string(),
        reply_scope: None,
        from: sender,
        to: Vec::new(),
        correlation_id: None,
        text: Some(text),
        attachments: Vec::new(),
        metadata,
    }
}

fn parse_query(query: &Option<String>) -> Option<HashMap<String, String>> {
    let query = query.as_deref()?;
    let mut map = HashMap::new();
    for pair in query.split('&') {
        let mut parts = pair.splitn(2, '=');
        if let (Some(key), Some(value)) = (parts.next(), parts.next()) {
            map.insert(key.to_string(), value.to_string());
        }
    }
    if map.is_empty() { None } else { Some(map) }
}

/// Serialize HttpOutV1 with "v":1 for operator v0.4.x compatibility.
fn http_out_v1_bytes(out: &HttpOutV1) -> Vec<u8> {
    let mut val = serde_json::to_value(out).unwrap_or(Value::Null);
    if let Some(map) = val.as_object_mut() {
        map.insert("v".to_string(), json!(1));
    }
    serde_json::to_vec(&val).unwrap_or_default()
}

fn http_out_error(status: u16, message: &str) -> Vec<u8> {
    let out = HttpOutV1 {
        status,
        headers: Vec::new(),
        body_b64: general_purpose::STANDARD.encode(message.as_bytes()),
        events: Vec::new(),
    };
    http_out_v1_bytes(&out)
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

#[cfg(test)]
fn parse_config_bytes(bytes: &[u8]) -> Result<ProviderConfig, String> {
    let cfg = serde_json::from_slice::<ProviderConfig>(bytes)
        .map_err(|e| format!("invalid config: {e}"))?;
    validate_provider_config(cfg)
}

fn parse_config_value(val: &Value) -> Result<ProviderConfig, String> {
    let cfg = serde_json::from_value::<ProviderConfig>(val.clone())
        .map_err(|e| format!("invalid config: {e}"))?;
    validate_provider_config(cfg)
}

fn load_config(input: &Value) -> Result<ProviderConfig, String> {
    if let Some(cfg) = input.get("config") {
        return parse_config_value(cfg);
    }
    let mut partial = serde_json::Map::new();
    for key in [
        "enabled",
        "phone_number_id",
        "public_base_url",
        "business_account_id",
        "api_base_url",
        "api_version",
        "token",
    ] {
        if let Some(v) = input.get(key) {
            partial.insert(key.to_string(), v.clone());
        }
    }
    if !partial.is_empty() {
        return parse_config_value(&Value::Object(partial));
    }

    Err("config required".into())
}

fn existing_config_from_answers(answers: &Value) -> Option<ProviderConfigOut> {
    answers
        .get("existing_config")
        .cloned()
        .or_else(|| answers.get("config").cloned())
        .and_then(|value| serde_json::from_value::<ProviderConfigOut>(value).ok())
}

fn optional_string_from(answers: &Value, key: &str) -> Option<String> {
    let value = answers.get(key)?;
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Value::Null => None,
        _ => None,
    }
}

fn string_or_default(answers: &Value, key: &str, default: &str) -> String {
    answers
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| default.to_string())
}

fn default_config_out() -> ProviderConfigOut {
    ProviderConfigOut {
        enabled: true,
        phone_number_id: String::new(),
        public_base_url: String::new(),
        business_account_id: None,
        api_base_url: DEFAULT_API_BASE.to_string(),
        api_version: DEFAULT_API_VERSION.to_string(),
        token: None,
    }
}

fn validate_config_out(config: &ProviderConfigOut) -> Result<(), String> {
    if config.phone_number_id.trim().is_empty() {
        return Err("config validation failed: phone_number_id is required".to_string());
    }
    if config.public_base_url.trim().is_empty() {
        return Err("config validation failed: public_base_url is required".to_string());
    }
    if !(config.public_base_url.starts_with("http://")
        || config.public_base_url.starts_with("https://"))
    {
        return Err(
            "config validation failed: public_base_url must be an absolute URL".to_string(),
        );
    }
    if !(config.api_base_url.starts_with("http://") || config.api_base_url.starts_with("https://"))
    {
        return Err("config validation failed: api_base_url must be an absolute URL".to_string());
    }
    Ok(())
}

fn json_bytes<T: serde::Serialize>(value: &T) -> Vec<u8> {
    serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec())
}

fn validate_provider_config(cfg: ProviderConfig) -> Result<ProviderConfig, String> {
    if cfg.phone_number_id.trim().is_empty() {
        return Err("invalid config: phone_number_id cannot be empty".to_string());
    }
    if cfg.public_base_url.trim().is_empty() {
        return Err("invalid config: public_base_url cannot be empty".to_string());
    }
    if let Some(business_account_id) = cfg.business_account_id.as_deref() {
        let _ = business_account_id.trim();
    }
    Ok(cfg)
}

fn get_token(cfg: &ProviderConfig) -> Result<String, String> {
    if let Some(token) = cfg.token.clone() {
        let token = token.trim().to_string();
        if !token.is_empty() {
            return Ok(token);
        }
    }
    match secrets_store::get(DEFAULT_TOKEN_KEY) {
        Ok(Some(bytes)) => {
            String::from_utf8(bytes).map_err(|_| "access_token not utf-8".to_string())
        }
        Ok(None) => Err(format!(
            "missing token (config or secret: {})",
            DEFAULT_TOKEN_KEY
        )),
        Err(e) => Err(format!("secret store error: {e:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn parse_config_requires_new_fields() {
        let cfg = br#"{"enabled":true,"phone_number_id":"pn","public_base_url":"https://example.com","api_base_url":"https://graph.facebook.com","api_version":"v19.0"}"#;
        let parsed = parse_config_bytes(cfg).expect("valid config");
        assert!(parsed.enabled);
        assert_eq!(parsed.phone_number_id, "pn");
    }

    #[test]
    fn parse_config_rejects_unknown() {
        let cfg = br#"{"enabled":true,"phone_number_id":"p","public_base_url":"https://example.com","api_base_url":"https://graph.facebook.com","api_version":"v19.0","unexpected":true}"#;
        let err = parse_config_bytes(cfg).unwrap_err();
        assert!(err.contains("unknown field"));
    }

    #[test]
    fn load_config_prefers_nested() {
        let input = json!({
            "config": {
                "enabled": true,
                "phone_number_id":"pn",
                "public_base_url":"https://example.com",
                "api_base_url":"https://graph.facebook.com",
                "api_version":"v20.0"
            },
            "api_version": "outer"
        });
        let cfg = load_config(&input).unwrap();
        assert_eq!(cfg.api_version.as_deref(), Some("v20.0"));
        assert_eq!(cfg.phone_number_id, "pn");
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

    #[test]
    fn qa_default_asks_required_minimum() {
        use bindings::exports::greentic::component::qa::Mode;
        let spec = build_qa_spec(Mode::Default);
        let keys = spec
            .questions
            .into_iter()
            .map(|question| question.key)
            .collect::<Vec<_>>();
        assert_eq!(keys, vec!["phone_number_id", "public_base_url"]);
    }

    #[test]
    fn apply_answers_upgrade_preserves_unspecified_fields() {
        use bindings::exports::greentic::component::qa::Guest as QaGuest;
        use bindings::exports::greentic::component::qa::Mode;
        let answers = json!({
            "existing_config": {
                "enabled": true,
                "phone_number_id": "123",
                "public_base_url": "https://example.com",
                "business_account_id": "old-business",
                "api_base_url": "https://graph.facebook.com",
                "api_version": "v19.0",
                "token": "token-a"
            },
            "business_account_id": "new-business"
        });
        let out =
            <Component as QaGuest>::apply_answers(Mode::Upgrade, canonical_cbor_bytes(&answers));
        let out_json: Value = decode_cbor(&out).expect("decode apply output");
        assert_eq!(out_json.get("ok"), Some(&Value::Bool(true)));
        let config = out_json.get("config").expect("config object");
        assert_eq!(
            config.get("phone_number_id"),
            Some(&Value::String("123".to_string()))
        );
        assert_eq!(
            config.get("business_account_id"),
            Some(&Value::String("new-business".to_string()))
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
            "phone_number_id": "123",
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
