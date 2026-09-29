//! Teams messaging provider component.
//!
//! Uses Microsoft Graph for outbound messaging.
//!
//! Implementation details are split across submodules:
//! - `auth`: Microsoft Graph token acquisition and legacy JWT helpers
//! - `config`: Configuration parsing and validation
//! - `describe`: Provider description and QA specs
//! - `ops`: Core operations (send, reply, render_plan, encode, send_payload)

use provider_common::component_v0_6::{canonical_cbor_bytes, decode_cbor};
use provider_common::helpers::json_bytes;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

mod bindings {
    wit_bindgen::generate!({
        path: "wit/messaging-provider-teams-graph",
        world: "component-v0-v6-v0",
        generate_all
    });
}

pub(crate) mod auth;
pub(crate) mod config;
mod describe;
mod ops;

/// Component-side operational log. The runner host forwards WASM stderr to
/// the operator console, so these lines surface next to the host's own
/// `[setup-action ...]` logs. Never log token/secret VALUES here —
/// presence/absence only.
pub(crate) fn wlog(msg: &str) {
    eprintln!("[messaging-teams] {msg}");
}

// Provider identification
pub(crate) const PROVIDER_ID: &str = "messaging-provider-teams-graph";
pub(crate) const PROVIDER_TYPE: &str = "messaging.teams.graph";
pub(crate) const WORLD_ID: &str = "component-v0-v6-v0";

// Microsoft Graph constants
pub(crate) const DEFAULT_GRAPH_TENANT_ID_KEY: &str = "MS_GRAPH_TENANT_ID";
pub(crate) const DEFAULT_GRAPH_CLIENT_ID_KEY: &str = "MS_GRAPH_CLIENT_ID";
pub(crate) const DEFAULT_GRAPH_REFRESH_TOKEN_KEY: &str = "MS_GRAPH_REFRESH_TOKEN";
pub(crate) const DEFAULT_GRAPH_CLIENT_SECRET_KEY: &str = "MS_GRAPH_CLIENT_SECRET";
pub(crate) const DEFAULT_GRAPH_ACCESS_TOKEN_KEY: &str = "MS_GRAPH_ACCESS_TOKEN";
pub(crate) const DEFAULT_GRAPH_BASE_URL: &str = "https://graph.microsoft.com/v1.0";
pub(crate) const DEFAULT_AUTH_BASE_URL: &str = "https://login.microsoftonline.com";
pub(crate) const DEFAULT_GRAPH_TOKEN_SCOPE: &str = "https://graph.microsoft.com/.default";

// Legacy Bot Service constants retained for old ingress/JWT compatibility.
pub(crate) const DEFAULT_BOT_APP_PASSWORD_KEY: &str = "MS_BOT_APP_PASSWORD";
pub(crate) const DEFAULT_BOT_TOKEN_ENDPOINT: &str =
    "https://login.microsoftonline.com/f28345e6-e7f5-4fff-b9b2-16baf26c13b4/oauth2/v2.0/token";
pub(crate) const DEFAULT_BOT_TOKEN_SCOPE: &str = "https://api.botframework.com/.default";

use config::{ProviderConfigOut, default_config_out, validate_config_out};
use describe::{
    DEFAULT_KEYS, I18N_KEYS, I18N_PAIRS, SETUP_QUESTIONS, build_describe_payload, build_qa_spec,
};
use ops::{
    encode_op, ensure_channel, handle_reply, handle_send, ingest_http, maybe_ensure_channel_config,
    render_plan, send_payload, send_typing,
};

// ============================================================================
// Component trait implementations
// ============================================================================

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
        apply_answers_impl(mode, answers_cbor)
    }
}

impl bindings::exports::greentic::component::component_i18n::Guest for Component {
    fn i18n_keys() -> Vec<String> {
        I18N_KEYS.iter().map(|k| (*k).to_string()).collect()
    }

    fn i18n_bundle(locale: String) -> Vec<u8> {
        describe::i18n_bundle(locale)
    }
}

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
        if let Some(result) = provider_common::qa_invoke_bridge::dispatch_qa_ops_with_i18n(
            &op,
            &input_json,
            "teams",
            SETUP_QUESTIONS,
            DEFAULT_KEYS,
            I18N_KEYS,
            I18N_PAIRS,
            apply_answers_bridge,
        ) {
            return result;
        }
        dispatch_json_invoke(&op, &input_json)
    }
}

impl bindings::exports::greentic::provider_instance_identity::instance_identity_api::Guest
    for Component
{
    fn identify_instance(input_json: Vec<u8>) -> Option<String> {
        ops::extract_recipient_id(&input_json)
    }
}

impl bindings::exports::greentic::provider_instance_identity::instance_identity_describe_api::Guest
    for Component
{
    fn describe_identify_instance() -> Option<Vec<u8>> {
        Some(ops::IDENTIFY_HINT_JSON.to_vec())
    }
}

bindings::export!(Component with_types_in bindings);

// ============================================================================
// Dispatch
// ============================================================================

fn apply_answers_bridge(mode: &str, answers_cbor: Vec<u8>) -> Vec<u8> {
    use bindings::exports::greentic::component::qa::Mode;
    let mode = match mode {
        "setup" => Mode::Setup,
        "upgrade" => Mode::Upgrade,
        "remove" => Mode::Remove,
        _ => Mode::Default,
    };
    apply_answers_impl(mode, answers_cbor)
}

fn dispatch_json_invoke(op: &str, input_json: &[u8]) -> Vec<u8> {
    match op {
        "run" | "send" => handle_send(input_json),
        "reply" => handle_reply(input_json),
        "ensure_channel" | "ensure-channel" => ensure_channel(input_json),
        "ingest_http" => ingest_http(input_json),
        "render_plan" => render_plan(input_json),
        "encode" => encode_op(input_json),
        "send_payload" => send_payload(input_json),
        "send_typing" => send_typing(input_json),
        // Note: subscription_* operations removed - Bot Service handles subscriptions automatically
        other => json_bytes(&json!({"ok": false, "error": format!("unsupported op: {other}")})),
    }
}

// ============================================================================
// QA apply_answers implementation
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ApplyAnswersResult {
    ok: bool,
    config: Option<ProviderConfigOut>,
    remove: Option<RemovePlan>,
    diagnostics: Vec<String>,
    error: Option<String>,
    #[serde(flatten)]
    runtime_answers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RemovePlan {
    remove_all: bool,
    cleanup: Vec<String>,
}

fn apply_answers_impl(
    mode: bindings::exports::greentic::component::qa::Mode,
    answers_cbor: Vec<u8>,
) -> Vec<u8> {
    use bindings::exports::greentic::component::qa::Mode;

    let answers: Value = match decode_cbor(&answers_cbor) {
        Ok(value) => value,
        Err(err) => {
            return canonical_cbor_bytes(&ApplyAnswersResult {
                ok: false,
                config: None,
                remove: None,
                diagnostics: Vec::new(),
                error: Some(format!("invalid answers cbor: {err}")),
                runtime_answers: BTreeMap::new(),
            });
        }
    };

    if mode == Mode::Remove {
        return canonical_cbor_bytes(&ApplyAnswersResult {
            ok: true,
            config: None,
            remove: Some(RemovePlan {
                remove_all: true,
                cleanup: vec![
                    "delete_config_key".to_string(),
                    "delete_provenance_key".to_string(),
                    "delete_provider_state_namespace".to_string(),
                    "best_effort_revoke_tokens".to_string(),
                    "best_effort_delete_provider_owned_secrets".to_string(),
                ],
            }),
            diagnostics: Vec::new(),
            error: None,
            runtime_answers: BTreeMap::new(),
        });
    }

    let mut merged = existing_config_from_answers(&answers).unwrap_or_else(default_config_out);
    let answer_obj = answers.as_object();
    let has = |key: &str| answer_obj.is_some_and(|obj| obj.contains_key(key));

    if mode == Mode::Setup || mode == Mode::Default {
        merged.enabled = answers
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(merged.enabled);
        merged.public_base_url =
            optional_string_from(&answers, "public_base_url").or(merged.public_base_url.clone());
        merged.setup_mode =
            optional_string_from(&answers, "setup_mode").or(merged.setup_mode.clone());
        merged.tenant_id = string_or_default(&answers, "tenant_id", &merged.tenant_id);
        merged.client_id = string_or_default(&answers, "client_id", &merged.client_id);
        merged.refresh_token =
            optional_string_from(&answers, "refresh_token").or(merged.refresh_token.clone());
        merged.client_secret =
            optional_string_from(&answers, "client_secret").or(merged.client_secret.clone());
        merged.access_token =
            optional_string_from(&answers, "access_token").or(merged.access_token.clone());
        merged.graph_base_url =
            string_or_default(&answers, "graph_base_url", &merged.graph_base_url);
        merged.auth_base_url = string_or_default(&answers, "auth_base_url", &merged.auth_base_url);
        merged.token_scope = string_or_default(&answers, "token_scope", &merged.token_scope);
        merged.team_id = optional_string_from(&answers, "team_id").or(merged.team_id.clone());
        merged.team_name = optional_string_from(&answers, "team_name").or(merged.team_name.clone());
        merged.channel_id =
            optional_string_from(&answers, "channel_id").or(merged.channel_id.clone());
        merged.channel_name =
            optional_string_from(&answers, "channel_name").or(merged.channel_name.clone());
        merged.desired_channel_name = optional_string_from(&answers, "desired_channel_name")
            .or_else(|| optional_string_from(&answers, "channel_name"))
            .or(merged.desired_channel_name.clone());
        merged.chat_id = optional_string_from(&answers, "chat_id").or(merged.chat_id.clone());
        merged.user_id = optional_string_from(&answers, "user_id").or(merged.user_id.clone());
        merged.ms_bot_app_id =
            optional_string_from(&answers, "ms_bot_app_id").or(merged.ms_bot_app_id.clone());
        merged.ms_bot_app_password = optional_string_from(&answers, "ms_bot_app_password")
            .or(merged.ms_bot_app_password.clone());
        merged.bot_display_name =
            optional_string_from(&answers, "bot_display_name").or(merged.bot_display_name.clone());
        merged.messaging_endpoint = optional_string_from(&answers, "messaging_endpoint")
            .or(merged.messaging_endpoint.clone());
    }

    if mode == Mode::Upgrade {
        if has("enabled") {
            merged.enabled = answers
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(merged.enabled);
        }
        if has("public_base_url") {
            merged.public_base_url = optional_string_from(&answers, "public_base_url");
        }
        if has("setup_mode") {
            merged.setup_mode = optional_string_from(&answers, "setup_mode");
        }
        if has("tenant_id") {
            merged.tenant_id = string_or_default(&answers, "tenant_id", &merged.tenant_id);
        }
        if has("client_id") {
            merged.client_id = string_or_default(&answers, "client_id", &merged.client_id);
        }
        if has("refresh_token") {
            merged.refresh_token = optional_string_from(&answers, "refresh_token");
        }
        if has("client_secret") {
            merged.client_secret = optional_string_from(&answers, "client_secret");
        }
        if has("access_token") {
            merged.access_token = optional_string_from(&answers, "access_token");
        }
        if has("graph_base_url") {
            merged.graph_base_url =
                string_or_default(&answers, "graph_base_url", &merged.graph_base_url);
        }
        if has("auth_base_url") {
            merged.auth_base_url =
                string_or_default(&answers, "auth_base_url", &merged.auth_base_url);
        }
        if has("token_scope") {
            merged.token_scope = string_or_default(&answers, "token_scope", &merged.token_scope);
        }
        if has("team_id") {
            merged.team_id = optional_string_from(&answers, "team_id");
        }
        if has("team_name") {
            merged.team_name = optional_string_from(&answers, "team_name");
        }
        if has("channel_id") {
            merged.channel_id = optional_string_from(&answers, "channel_id");
        }
        if has("channel_name") {
            merged.channel_name = optional_string_from(&answers, "channel_name");
        }
        if has("desired_channel_name") {
            merged.desired_channel_name = optional_string_from(&answers, "desired_channel_name");
        }
        if has("chat_id") {
            merged.chat_id = optional_string_from(&answers, "chat_id");
        }
        if has("user_id") {
            merged.user_id = optional_string_from(&answers, "user_id");
        }
        if has("ms_bot_app_id") {
            merged.ms_bot_app_id = optional_string_from(&answers, "ms_bot_app_id");
        }
        if has("ms_bot_app_password") {
            merged.ms_bot_app_password = optional_string_from(&answers, "ms_bot_app_password");
        }
        if has("bot_display_name") {
            merged.bot_display_name = optional_string_from(&answers, "bot_display_name");
        }
        if has("messaging_endpoint") {
            merged.messaging_endpoint = optional_string_from(&answers, "messaging_endpoint");
        }
    }

    let channel_created = if mode == Mode::Setup || mode == Mode::Default {
        match maybe_ensure_channel_config(&mut merged) {
            Ok(created) => created,
            Err(error) => {
                return canonical_cbor_bytes(&ApplyAnswersResult {
                    ok: false,
                    config: None,
                    remove: None,
                    diagnostics: Vec::new(),
                    error: Some(error),
                    runtime_answers: BTreeMap::new(),
                });
            }
        }
    } else {
        false
    };

    if let Err(error) = validate_config_out(&merged) {
        return canonical_cbor_bytes(&ApplyAnswersResult {
            ok: false,
            config: None,
            remove: None,
            diagnostics: Vec::new(),
            error: Some(error),
            runtime_answers: BTreeMap::new(),
        });
    }

    canonical_cbor_bytes(&ApplyAnswersResult {
        ok: true,
        runtime_answers: runtime_answers_from_config(&merged),
        config: Some(merged),
        remove: None,
        diagnostics: if channel_created {
            vec!["created Teams channel from desired_channel_name".to_string()]
        } else {
            Vec::new()
        },
        error: None,
    })
}

fn runtime_answers_from_config(config: &ProviderConfigOut) -> BTreeMap<String, String> {
    let mut answers = BTreeMap::new();
    insert_non_empty(&mut answers, "tenant_id", Some(config.tenant_id.as_str()));
    insert_non_empty(&mut answers, "client_id", Some(config.client_id.as_str()));
    insert_non_empty(&mut answers, "team_id", config.team_id.as_deref());
    insert_non_empty(&mut answers, "team_name", config.team_name.as_deref());
    insert_non_empty(&mut answers, "channel_id", config.channel_id.as_deref());
    insert_non_empty(&mut answers, "channel_name", config.channel_name.as_deref());
    insert_non_empty(
        &mut answers,
        "desired_channel_name",
        config.desired_channel_name.as_deref(),
    );
    insert_non_empty(&mut answers, "chat_id", config.chat_id.as_deref());
    insert_non_empty(&mut answers, "user_id", config.user_id.as_deref());
    answers
}

fn insert_non_empty(map: &mut BTreeMap<String, String>, key: &str, value: Option<&str>) {
    if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
        map.insert(key.to_string(), value.to_string());
    }
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

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use config::parse_config_bytes;
    use provider_common::component_v0_6::schema_hash;
    use std::collections::BTreeSet;

    #[test]
    fn send_typing_is_dispatched_not_unsupported() {
        let out: Value =
            serde_json::from_slice(&dispatch_json_invoke("send_typing", b"{")).expect("json");
        assert_eq!(out["ok"], false);
        assert!(
            !out["error"]
                .as_str()
                .unwrap_or("")
                .contains("unsupported op")
        );
    }

    #[test]
    fn parse_config_requires_new_fields() {
        let cfg = br#"{"enabled":true,"tenant_id":"tenant","client_id":"client","refresh_token":"refresh"}"#;
        let parsed = parse_config_bytes(cfg).expect("valid config");
        assert!(parsed.enabled);
        assert_eq!(parsed.tenant_id, "tenant");
        assert_eq!(parsed.client_id, "client");
    }

    #[test]
    fn load_config_prefers_nested_config() {
        let input = json!({
            "config": {
                "enabled": true,
                "tenant_id": "tenant",
                "client_id": "client",
                "refresh_token": "refresh",
                "team_id": "team-abc"
            },
        });
        let cfg = config::load_config(&input).expect("config");
        assert_eq!(cfg.team_id.as_deref(), Some("team-abc"));
    }

    #[test]
    fn parse_config_rejects_unknown() {
        let cfg = br#"{"enabled":true,"tenant_id":"tenant","client_id":"client","refresh_token":"refresh","unknown":"field"}"#;
        let err = parse_config_bytes(cfg).unwrap_err();
        assert!(err.contains("unknown field"));
    }

    #[test]
    fn schema_hash_is_stable() {
        let describe = build_describe_payload();
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
    fn qa_default_uses_device_code_action_without_manual_questions() {
        use bindings::exports::greentic::component::qa::Mode;
        let spec = build_qa_spec(Mode::Default);
        let keys = spec
            .questions
            .into_iter()
            .map(|question| question.id)
            .collect::<Vec<_>>();
        assert!(keys.is_empty());
    }

    #[test]
    fn apply_answers_upgrade_preserves_unspecified_fields() {
        use bindings::exports::greentic::component::qa::Guest as QaGuest;
        use bindings::exports::greentic::component::qa::Mode;
        let answers = json!({
            "existing_config": {
                "enabled": true,
                "tenant_id": "tenant",
                "client_id": "client",
                "refresh_token": "secret-a",
                "team_id": "team-123",
                "graph_base_url": "https://graph.microsoft.com/v1.0",
                "auth_base_url": "https://login.microsoftonline.com",
                "token_scope": "https://graph.microsoft.com/.default"
            },
            "team_id": "team-456"
        });
        let out =
            <Component as QaGuest>::apply_answers(Mode::Upgrade, canonical_cbor_bytes(&answers));
        let out_json: Value = decode_cbor(&out).expect("decode apply output");
        assert_eq!(out_json.get("ok"), Some(&Value::Bool(true)));
        let config = out_json.get("config").expect("config object");
        assert_eq!(config.get("ms_bot_app_password"), None);
        assert_eq!(
            config.get("refresh_token"),
            Some(&Value::String("secret-a".to_string()))
        );
        assert_eq!(
            config.get("team_id"),
            Some(&Value::String("team-456".to_string()))
        );
    }

    #[test]
    fn apply_answers_persists_graph_selection_labels() {
        use bindings::exports::greentic::component::qa::Guest as QaGuest;
        use bindings::exports::greentic::component::qa::Mode;
        let answers = json!({
            "setup_mode": "graph_channel",
            "tenant_id": "tenant",
            "client_id": "client",
            "access_token": "token",
            "team_id": "team-123",
            "team_name": "Engineering",
            "channel_id": "19:general@thread.tacv2",
            "channel_name": "General",
            "desired_channel_name": "Engineering Updates"
        });

        let out = crate::ops::with_http_send_mock(
            |req| {
                if req.method == "GET" {
                    assert_eq!(
                        req.url,
                        "https://graph.microsoft.com/v1.0/teams/team-123/channels"
                    );
                    return Ok(crate::bindings::greentic::http::http_client::Response {
                        status: 200,
                        headers: vec![],
                        body: Some(br#"{"value":[{"id":"19:general@thread.tacv2","displayName":"General"}]}"#.to_vec()),
                    });
                }
                assert_eq!(req.method, "POST");
                assert_eq!(
                    req.url,
                    "https://graph.microsoft.com/v1.0/teams/team-123/channels"
                );
                let body: Value = serde_json::from_slice(req.body.as_deref().unwrap_or_default())
                    .expect("create body");
                assert_eq!(body["displayName"], "Engineering Updates");
                Ok(crate::bindings::greentic::http::http_client::Response {
                    status: 201,
                    headers: vec![],
                    body: Some(
                        br#"{"id":"19:engineering-updates@thread.tacv2","displayName":"Engineering Updates"}"#.to_vec(),
                    ),
                })
            },
            || <Component as QaGuest>::apply_answers(Mode::Default, canonical_cbor_bytes(&answers)),
        );
        let out_json: Value = decode_cbor(&out).expect("decode apply output");
        assert_eq!(out_json.get("ok"), Some(&Value::Bool(true)));
        let config = out_json.get("config").expect("config object");
        assert_eq!(
            config.get("team_id"),
            Some(&Value::String("team-123".to_string()))
        );
        assert_eq!(
            config.get("team_name"),
            Some(&Value::String("Engineering".to_string()))
        );
        assert_eq!(
            config.get("channel_id"),
            Some(&Value::String(
                "19:engineering-updates@thread.tacv2".to_string()
            ))
        );
        assert_eq!(
            config.get("channel_name"),
            Some(&Value::String("Engineering Updates".to_string()))
        );
        assert_eq!(
            config.get("desired_channel_name"),
            Some(&Value::String("Engineering Updates".to_string()))
        );
        assert_eq!(
            out_json.get("diagnostics"),
            Some(&json!(["created Teams channel from desired_channel_name"]))
        );
    }

    #[test]
    fn apply_answers_exposes_subscription_source_fields_at_top_level() {
        use bindings::exports::greentic::component::qa::Guest as QaGuest;
        use bindings::exports::greentic::component::qa::Mode;
        let answers = json!({
            "setup_mode": "graph_channel",
            "tenant_id": "tenant",
            "client_id": "client",
            "team_id": "team-123",
            "team_name": "Engineering",
            "channel_id": "19:general@thread.tacv2",
            "channel_name": "General",
            "chat_id": "chat-123"
        });

        let out =
            <Component as QaGuest>::apply_answers(Mode::Default, canonical_cbor_bytes(&answers));
        let out_json: Value = decode_cbor(&out).expect("decode apply output");
        assert_eq!(out_json.get("ok"), Some(&Value::Bool(true)));
        assert_eq!(out_json["tenant_id"], "tenant");
        assert_eq!(out_json["client_id"], "client");
        assert_eq!(out_json["team_id"], "team-123");
        assert_eq!(out_json["team_name"], "Engineering");
        assert_eq!(out_json["channel_id"], "19:general@thread.tacv2");
        assert_eq!(out_json["channel_name"], "General");
        assert_eq!(out_json["chat_id"], "chat-123");
    }

    #[test]
    fn apply_answers_reuses_existing_channel_matching_desired_name() {
        use bindings::exports::greentic::component::qa::Guest as QaGuest;
        use bindings::exports::greentic::component::qa::Mode;
        let answers = json!({
            "setup_mode": "graph_channel",
            "tenant_id": "tenant",
            "client_id": "client",
            "access_token": "token",
            "team_id": "team-123",
            "team_name": "Engineering",
            "channel_id": "19:general@thread.tacv2",
            "channel_name": "General",
            "desired_channel_name": "Engineering Updates"
        });

        let out = crate::ops::with_http_send_mock(
            |req| {
                assert_eq!(req.method, "GET", "existing channel should be reused");
                assert_eq!(
                    req.url,
                    "https://graph.microsoft.com/v1.0/teams/team-123/channels"
                );
                Ok(crate::bindings::greentic::http::http_client::Response {
                    status: 200,
                    headers: vec![],
                    body: Some(
                        br#"{"value":[{"id":"19:engineering-updates@thread.tacv2","displayName":"Engineering Updates"}]}"#.to_vec(),
                    ),
                })
            },
            || <Component as QaGuest>::apply_answers(Mode::Default, canonical_cbor_bytes(&answers)),
        );
        let out_json: Value = decode_cbor(&out).expect("decode apply output");
        assert_eq!(out_json.get("ok"), Some(&Value::Bool(true)));
        let config = out_json.get("config").expect("config object");
        assert_eq!(
            config.get("channel_id"),
            Some(&Value::String(
                "19:engineering-updates@thread.tacv2".to_string()
            ))
        );
        assert_eq!(
            config.get("channel_name"),
            Some(&Value::String("Engineering Updates".to_string()))
        );
        assert_eq!(out_json.get("diagnostics"), Some(&json!([])));
    }

    #[test]
    fn apply_answers_accepts_channel_name_as_default_hint_without_channel_id_override() {
        use bindings::exports::greentic::component::qa::Guest as QaGuest;
        use bindings::exports::greentic::component::qa::Mode;
        let answers = json!({
            "tenant_id": "tenant",
            "client_id": "client",
            "team_id": "team-id",
            "channel_id": "channel-id",
            "channel_name": "Bundle Display Name"
        });

        let out =
            <Component as QaGuest>::apply_answers(Mode::Default, canonical_cbor_bytes(&answers));
        let out_json: Value = decode_cbor(&out).expect("decode apply output");
        let config = out_json.get("config").expect("config object");
        assert_eq!(
            config.get("channel_id"),
            Some(&Value::String("channel-id".to_string()))
        );
        assert_eq!(
            config.get("channel_name"),
            Some(&Value::String("Bundle Display Name".to_string()))
        );
        assert_eq!(
            config.get("desired_channel_name"),
            Some(&Value::String("Bundle Display Name".to_string()))
        );
    }

    #[test]
    fn apply_answers_accepts_bot_framework_config_shape() {
        use bindings::exports::greentic::component::qa::Guest as QaGuest;
        use bindings::exports::greentic::component::qa::Mode;
        let answers = json!({
            "setup_mode": "bot_framework",
            "tenant_id": "tenant",
            "client_id": "bot-app-id",
            "ms_bot_app_id": "bot-app-id",
            "ms_bot_app_password": "bot-secret",
            "bot_display_name": "Greentic Bot",
            "messaging_endpoint": "https://example.com/api/messages"
        });

        let out =
            <Component as QaGuest>::apply_answers(Mode::Default, canonical_cbor_bytes(&answers));
        let out_json: Value = decode_cbor(&out).expect("decode apply output");
        assert_eq!(out_json.get("ok"), Some(&Value::Bool(true)));
        assert_eq!(
            out_json
                .get("config")
                .and_then(|c| c.get("setup_mode"))
                .and_then(Value::as_str),
            Some("bot_framework")
        );
    }

    #[test]
    fn apply_answers_rejects_bot_framework_without_app_id() {
        use bindings::exports::greentic::component::qa::Guest as QaGuest;
        use bindings::exports::greentic::component::qa::Mode;
        let answers = json!({
            "setup_mode": "bot_framework",
            "tenant_id": "tenant",
            "client_id": "client"
        });

        let out =
            <Component as QaGuest>::apply_answers(Mode::Default, canonical_cbor_bytes(&answers));
        let out_json: Value = decode_cbor(&out).expect("decode apply output");
        assert_eq!(out_json.get("ok"), Some(&Value::Bool(false)));
        assert_eq!(
            out_json.get("error").and_then(Value::as_str),
            Some("config validation failed: ms_bot_app_id is required for bot_framework mode")
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
            "ms_bot_app_id": "app-id",
            "tenant_id": "tenant",
            "client_id": "client",
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
