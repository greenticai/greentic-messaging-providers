use base64::{Engine as _, engine::general_purpose};
use greentic_types::messaging::universal_dto::{
    EncodeInV1, HttpInV1, HttpOutV1, ProviderPayloadV1, RenderPlanInV1, RenderPlanOutV1,
    SendPayloadInV1, SendPayloadResultV1,
};
use greentic_types::{Actor, ChannelMessageEnvelope, EnvId, MessageMetadata, TenantCtx, TenantId};
use provider_common::component_v0_6::{
    DescribePayload, I18nText, OperationDescriptor, QaQuestionSpec, QaSpec, SchemaField, SchemaIr,
    canonical_cbor_bytes, decode_cbor, default_en_i18n_messages, schema_hash,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

mod bindings {
    wit_bindgen::generate!({
        path: "wit/messaging-provider-webchat",
        world: "component-v0-v6-v0",
        generate_all
    });
}
mod directline;

use bindings::greentic::state::state_store;
use directline::{HostSecretStore, HostStateStore, handle_directline_request};

const PROVIDER_ID: &str = "messaging-provider-webchat";
const PROVIDER_TYPE: &str = "messaging.webchat";
const WORLD_ID: &str = "component-v0-v6-v0";
const I18N_KEYS: &[&str] = &[
    "webchat.op.run.title",
    "webchat.op.run.description",
    "webchat.op.send.title",
    "webchat.op.send.description",
    "webchat.op.ingest.title",
    "webchat.op.ingest.description",
    "webchat.op.ingest_http.title",
    "webchat.op.ingest_http.description",
    "webchat.op.render_plan.title",
    "webchat.op.render_plan.description",
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

#[derive(Debug, Deserialize)]
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
    config: Option<ProviderConfigOut>,
    remove: Option<RemovePlan>,
    diagnostics: Vec<String>,
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProviderConfigOut {
    enabled: bool,
    public_base_url: String,
    mode: String,
    route: Option<String>,
    tenant_channel_id: Option<String>,
    base_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RemovePlan {
    remove_all: bool,
    cleanup: Vec<String>,
}

fn default_enabled() -> bool {
    true
}
fn default_mode() -> String {
    "local_queue".to_string()
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
            merged.mode = string_or_default(&answers, "mode", &merged.mode);
            merged.route = optional_string_from(&answers, "route").or(merged.route.clone());
            merged.tenant_channel_id = optional_string_from(&answers, "tenant_channel_id")
                .or(merged.tenant_channel_id.clone());
            merged.base_url =
                optional_string_from(&answers, "base_url").or(merged.base_url.clone());
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
            if has("mode") {
                merged.mode = string_or_default(&answers, "mode", &merged.mode);
            }
            if has("route") {
                merged.route = optional_string_from(&answers, "route");
            }
            if has("tenant_channel_id") {
                merged.tenant_channel_id = optional_string_from(&answers, "tenant_channel_id");
            }
            if has("base_url") {
                merged.base_url = optional_string_from(&answers, "base_url");
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

fn dispatch_json_invoke(op: &str, input_json: &[u8]) -> Vec<u8> {
    match op {
        "send" => handle_send(input_json),
        "ingest" => handle_ingest(input_json),
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
            op("run", "webchat.op.run.title", "webchat.op.run.description"),
            op(
                "send",
                "webchat.op.send.title",
                "webchat.op.send.description",
            ),
            op(
                "ingest",
                "webchat.op.ingest.title",
                "webchat.op.ingest.description",
            ),
            op(
                "ingest_http",
                "webchat.op.ingest_http.title",
                "webchat.op.ingest_http.description",
            ),
            op(
                "render_plan",
                "webchat.op.render_plan.title",
                "webchat.op.render_plan.description",
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
        redactions: Vec::new(),
        schema_hash: schema_hash(&input_schema, &output_schema, &config_schema),
    }
}

fn build_qa_spec(mode: bindings::exports::greentic::component::qa::Mode) -> QaSpec {
    use bindings::exports::greentic::component::qa::Mode;
    match mode {
        Mode::Default => QaSpec {
            mode: "default".to_string(),
            title: i18n("webchat.qa.default.title"),
            questions: vec![qa_q(
                "public_base_url",
                "webchat.qa.setup.public_base_url",
                true,
            )],
        },
        Mode::Setup => QaSpec {
            mode: "setup".to_string(),
            title: i18n("webchat.qa.setup.title"),
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
        },
        Mode::Upgrade => QaSpec {
            mode: "upgrade".to_string(),
            title: i18n("webchat.qa.upgrade.title"),
            questions: vec![
                qa_q("enabled", "webchat.qa.setup.enabled", false),
                qa_q("public_base_url", "webchat.qa.setup.public_base_url", false),
                qa_q("mode", "webchat.qa.setup.mode", false),
                qa_q("route", "webchat.qa.setup.route", false),
                qa_q(
                    "tenant_channel_id",
                    "webchat.qa.setup.tenant_channel_id",
                    false,
                ),
                qa_q("base_url", "webchat.qa.setup.base_url", false),
            ],
        },
        Mode::Remove => QaSpec {
            mode: "remove".to_string(),
            title: i18n("webchat.qa.remove.title"),
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
    let mut insert = |k: &str, required: bool, schema: SchemaIr| {
        fields.insert(k.to_string(), SchemaField { required, schema });
    };
    insert(
        "enabled",
        true,
        SchemaIr::Bool {
            title: i18n("webchat.schema.config.enabled.title"),
            description: i18n("webchat.schema.config.enabled.description"),
        },
    );
    insert(
        "public_base_url",
        true,
        SchemaIr::String {
            title: i18n("webchat.schema.config.public_base_url.title"),
            description: i18n("webchat.schema.config.public_base_url.description"),
            format: Some("uri".to_string()),
            secret: false,
        },
    );
    insert(
        "mode",
        true,
        SchemaIr::String {
            title: i18n("webchat.schema.config.mode.title"),
            description: i18n("webchat.schema.config.mode.description"),
            format: None,
            secret: false,
        },
    );
    insert(
        "route",
        false,
        SchemaIr::String {
            title: i18n("webchat.schema.config.route.title"),
            description: i18n("webchat.schema.config.route.description"),
            format: None,
            secret: false,
        },
    );
    insert(
        "tenant_channel_id",
        false,
        SchemaIr::String {
            title: i18n("webchat.schema.config.tenant_channel_id.title"),
            description: i18n("webchat.schema.config.tenant_channel_id.description"),
            format: None,
            secret: false,
        },
    );
    insert(
        "base_url",
        false,
        SchemaIr::String {
            title: i18n("webchat.schema.config.base_url.title"),
            description: i18n("webchat.schema.config.base_url.description"),
            format: Some("uri".to_string()),
            secret: false,
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

    let cfg = match load_config(&parsed) {
        Ok(cfg) => cfg,
        Err(err) => return json_bytes(&json!({"ok": false, "error": err})),
    };
    if !cfg.enabled {
        return json_bytes(&json!({"ok": false, "error": "provider disabled by config"}));
    }

    let route = parsed
        .get("route")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| cfg.route.clone());
    let tenant_channel_id = parsed
        .get("tenant_channel_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| cfg.tenant_channel_id.clone());

    if route.is_none() && tenant_channel_id.is_none() {
        return json_bytes(&json!({"ok": false, "error": "route or tenant_channel_id required"}));
    }

    let text = parsed
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if text.is_empty() {
        return json_bytes(&json!({"ok": false, "error": "text required"}));
    }

    let payload = json!({
        "route": route,
        "tenant_channel_id": tenant_channel_id,
        "public_base_url": cfg.public_base_url,
        "mode": cfg.mode,
        "base_url": cfg.base_url,
        "text": text,
    });
    let payload_bytes = json_bytes(&payload);
    let key = route
        .clone()
        .or(tenant_channel_id.clone())
        .unwrap_or_else(|| "webchat".to_string());

    let write_result = state_store::write(&key, &payload_bytes, None);
    if let Err(err) = write_result {
        return json_bytes(
            &json!({"ok": false, "error": format!("state write error: {}", err.message)}),
        );
    }

    let hash_hex = hex_sha256(&payload_bytes);
    let message_id = pseudo_uuid_from_hex(&hash_hex);
    let provider_message_id = format!("webchat:{hash_hex}");

    json_bytes(&json!({
        "ok": true,
        "status": "sent",
        "provider_type": PROVIDER_TYPE,
        "public_base_url": cfg.public_base_url,
        "message_id": message_id,
        "provider_message_id": provider_message_id,
        "payload": payload
    }))
}

fn handle_ingest(input_json: &[u8]) -> Vec<u8> {
    let parsed: Value = match serde_json::from_slice(input_json) {
        Ok(val) => val,
        Err(err) => {
            return json_bytes(&json!({"ok": false, "error": format!("invalid json: {err}")}));
        }
    };
    let text = parsed
        .get("text")
        .or_else(|| parsed.get("message"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let user = parsed
        .get("user_id")
        .or_else(|| parsed.get("from"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let envelope = json!({
        "from": user,
        "text": text,
        "raw": parsed,
    });
    json_bytes(&json!({"ok": true, "envelope": envelope}))
}

fn ingest_http(input_json: &[u8]) -> Vec<u8> {
    let request = match serde_json::from_slice::<HttpInV1>(input_json) {
        Ok(req) => req,
        Err(err) => return http_out_error(400, &format!("invalid http input: {err}")),
    };
    if request.path.starts_with("/v3/directline") {
        let mut state_driver = HostStateStore;
        let secrets_driver = HostSecretStore;
        let out = handle_directline_request(&request, &mut state_driver, &secrets_driver);
        return http_out_v1_bytes(&out);
    }
    let body_bytes = match general_purpose::STANDARD.decode(&request.body_b64) {
        Ok(bytes) => bytes,
        Err(err) => return http_out_error(400, &format!("invalid body encoding: {err}")),
    };
    let body_val: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    let text = extract_text(&body_val);
    let user = user_from_value(&body_val);
    let route =
        non_empty_string(request.route_hint.as_deref()).or_else(|| route_from_value(&body_val));
    let tenant_channel_id = tenant_channel_from_value(&body_val);
    let envelope = build_webchat_envelope(
        text.clone(),
        user.clone(),
        route.clone(),
        tenant_channel_id.clone(),
    );
    let normalized = json!({
        "ok": true,
        "event": body_val,
        "route": route,
        "tenant_channel_id": tenant_channel_id,
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
    let has_ac = plan_in.message.metadata.contains_key("adaptive_card");
    let tier = if has_ac { "TierA" } else { "TierD" };
    let summary = plan_in
        .message
        .text
        .clone()
        .filter(|text| !text.trim().is_empty())
        .unwrap_or_else(|| "webchat message".to_string());
    let plan_obj = json!({
        "tier": tier,
        "summary_text": summary,
        "actions": [],
        "attachments": [],
        "warnings": [],
        "debug": plan_in.metadata,
    });
    let plan_json =
        serde_json::to_string(&plan_obj).unwrap_or_else(|_| format!("{{\"tier\":\"{tier}\"}}"));
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
        .unwrap_or_else(|| "webchat universal payload".to_string());
    let metadata_route = encode_in.message.metadata.get("route").cloned();
    let route = metadata_route
        .clone()
        .or_else(|| Some(encode_in.message.session_id.clone()));
    let route_value = route.clone().unwrap_or_else(|| "webchat".to_string());
    let payload_body = json!({
        "text": text,
        "route": route_value.clone(),
        "session_id": encode_in.message.session_id,
    });
    let body_bytes = serde_json::to_vec(&payload_body).unwrap_or_else(|_| b"{}".to_vec());
    let mut metadata = BTreeMap::new();
    metadata.insert("route".to_string(), Value::String(route_value.clone()));
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
    match persist_send_payload(&payload) {
        Ok(_) => send_payload_success(),
        Err(err) => send_payload_error(&err, false),
    }
}

fn persist_send_payload(payload: &Value) -> Result<(), String> {
    let route = route_from_value(payload);
    let tenant_channel_id = tenant_channel_from_value(payload);
    let key = route
        .clone()
        .or(tenant_channel_id.clone())
        .ok_or_else(|| "route or tenant_channel_id required".to_string())?;
    let text = extract_text(payload);
    if text.is_empty() {
        return Err("text required".into());
    }
    let public_base_url = public_base_url_from_value(payload);
    let stored = json!({
        "route": route,
        "tenant_channel_id": tenant_channel_id,
        "public_base_url": public_base_url,
        "mode": value_as_trimmed_string(payload.get("mode")).unwrap_or_else(|| "local_queue".to_string()),
        "base_url": value_as_trimmed_string(payload.get("base_url")),
        "text": text,
    });
    state_store::write(&key, &json_bytes(&stored), None)
        .map_err(|err| format!("state write error: {}", err.message))?;
    Ok(())
}

fn build_webchat_envelope(
    text: String,
    user_id: Option<String>,
    route: Option<String>,
    tenant_channel_id: Option<String>,
) -> ChannelMessageEnvelope {
    let env = EnvId::try_from("default").expect("env id");
    let tenant = TenantId::try_from("default").expect("tenant id");
    let mut metadata = MessageMetadata::new();
    metadata.insert("universal".to_string(), "true".to_string());
    if let Some(route) = &route {
        metadata.insert("route".to_string(), route.clone());
    }
    if let Some(channel) = &tenant_channel_id {
        metadata.insert("tenant_channel_id".to_string(), channel.clone());
    }
    let channel = route
        .clone()
        .or_else(|| tenant_channel_id.clone())
        .unwrap_or_else(|| "webchat".to_string());
    ChannelMessageEnvelope {
        id: format!("webchat-{channel}"),
        tenant: TenantCtx::new(env.clone(), tenant.clone()),
        channel: channel.clone(),
        session_id: channel,
        reply_scope: None,
        from: user_id.map(|id| Actor {
            id,
            kind: Some("user".into()),
        }),
        to: Vec::new(),
        correlation_id: None,
        text: Some(text),
        attachments: Vec::new(),
        metadata,
    }
}

fn extract_text(value: &Value) -> String {
    value
        .get("text")
        .or_else(|| value.get("message"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn user_from_value(value: &Value) -> Option<String> {
    value
        .get("user_id")
        .or_else(|| value.get("from"))
        .and_then(|v| v.as_str())
        .and_then(|s| non_empty_string(Some(s)))
}

fn route_from_value(value: &Value) -> Option<String> {
    value_as_trimmed_string(value.get("route"))
}

fn tenant_channel_from_value(value: &Value) -> Option<String> {
    value_as_trimmed_string(value.get("tenant_channel_id"))
}

fn public_base_url_from_value(value: &Value) -> Option<String> {
    value_as_trimmed_string(value.get("public_base_url"))
}

fn value_as_trimmed_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

fn non_empty_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
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
        "public_base_url",
        "mode",
        "route",
        "tenant_channel_id",
        "base_url",
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
        public_base_url: String::new(),
        mode: default_mode(),
        route: None,
        tenant_channel_id: None,
        base_url: None,
    }
}

fn validate_config_out(config: &ProviderConfigOut) -> Result<(), String> {
    if config.public_base_url.trim().is_empty() {
        return Err("config validation failed: public_base_url is required".to_string());
    }
    if config.mode.trim().is_empty() {
        return Err("config validation failed: mode is required".to_string());
    }
    if !(config.public_base_url.starts_with("http://")
        || config.public_base_url.starts_with("https://"))
    {
        return Err(
            "config validation failed: public_base_url must be an absolute URL".to_string(),
        );
    }
    Ok(())
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(&mut out, "{:02x}", byte);
    }
    out
}

fn pseudo_uuid_from_hex(hex: &str) -> String {
    let padded = if hex.len() < 32 {
        format!("{hex:0<32}")
    } else {
        hex[..32].to_string()
    };
    format!(
        "{}-{}-{}-{}-{}",
        &padded[0..8],
        &padded[8..12],
        &padded[12..16],
        &padded[16..20],
        &padded[20..32]
    )
}

fn json_bytes<T: serde::Serialize>(value: &T) -> Vec<u8> {
    serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec())
}

fn validate_provider_config(cfg: ProviderConfig) -> Result<ProviderConfig, String> {
    if cfg.public_base_url.trim().is_empty() {
        return Err("invalid config: public_base_url cannot be empty".to_string());
    }
    let mode = cfg.mode.trim();
    if mode != "local_queue" && mode != "websocket" && mode != "pubsub" {
        return Err("invalid config: mode must be local_queue|websocket|pubsub".to_string());
    }
    if cfg.route.is_none() && cfg.tenant_channel_id.is_none() {
        return Err("invalid config: route or tenant_channel_id required".to_string());
    }
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn parse_config_requires_new_fields() {
        let cfg = br#"{"enabled":true,"public_base_url":"https://example.com","mode":"local_queue","route":"r"}"#;
        let parsed = parse_config_bytes(cfg).expect("valid config");
        assert!(parsed.enabled);
        assert_eq!(parsed.mode, "local_queue");
    }

    #[test]
    fn load_config_prefers_nested() {
        let input = json!({
            "config": {"enabled":true,"route":"inner","public_base_url":"https://example.com","mode":"local_queue"},
            "route": "outer"
        });
        let cfg = load_config(&input).unwrap();
        assert_eq!(cfg.route.as_deref(), Some("inner"));
        assert_eq!(cfg.public_base_url, "https://example.com");
    }

    #[test]
    fn parse_config_rejects_unknown() {
        let cfg = br#"{"enabled":true,"route":"r","public_base_url":"https://example.com","mode":"local_queue","extra":true}"#;
        let err = parse_config_bytes(cfg).unwrap_err();
        assert!(err.contains("unknown field"));
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
                "mode": "local_queue",
                "route": "/chat",
                "tenant_channel_id": "tenant-a",
                "base_url": "https://chat.example.com"
            },
            "route": "/messages"
        });
        let out =
            <Component as QaGuest>::apply_answers(Mode::Upgrade, canonical_cbor_bytes(&answers));
        let out_json: Value = decode_cbor(&out).expect("decode apply output");
        assert_eq!(out_json.get("ok"), Some(&Value::Bool(true)));
        let config = out_json.get("config").expect("config object");
        assert_eq!(
            config.get("tenant_channel_id"),
            Some(&Value::String("tenant-a".to_string()))
        );
        assert_eq!(
            config.get("route"),
            Some(&Value::String("/messages".to_string()))
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
