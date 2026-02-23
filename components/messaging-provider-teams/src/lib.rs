use provider_common::component_v0_6::{canonical_cbor_bytes, decode_cbor};
use provider_common::helpers::{
    cbor_json_invoke_bridge, existing_config_from_answers, json_bytes, optional_string_from,
    schema_core_describe, schema_core_healthcheck, schema_core_validate_config, string_or_default,
};
use provider_common::qa_helpers::ApplyAnswersResult;
use serde_json::{Value, json};

mod bindings {
    wit_bindgen::generate!({
        path: "wit/messaging-provider-teams",
        world: "component-v0-v6-v0",
        generate_all
    });
}

mod config;
mod describe;
mod graph;
mod ops;
mod token;

use config::{ProviderConfigOut, default_config_out, validate_config_out};
use describe::{I18N_KEYS, build_describe_payload, build_qa_spec};
use graph::{subscription_delete, subscription_ensure, subscription_renew};
use ops::{encode_op, handle_reply, handle_send, ingest_http, render_plan, send_payload};

const PROVIDER_ID: &str = "messaging-provider-teams";
const PROVIDER_TYPE: &str = "messaging.teams.graph";
const WORLD_ID: &str = "component-v0-v6-v0";
const DEFAULT_CLIENT_SECRET_KEY: &str = "MS_GRAPH_CLIENT_SECRET";
const DEFAULT_REFRESH_TOKEN_KEY: &str = "MS_GRAPH_REFRESH_TOKEN";
const DEFAULT_TENANT_ID_KEY: &str = "MS_GRAPH_TENANT_ID";
const DEFAULT_CLIENT_ID_KEY: &str = "MS_GRAPH_CLIENT_ID";
const DEFAULT_TOKEN_SCOPE: &str = "https://graph.microsoft.com/.default";
const DEFAULT_GRAPH_BASE: &str = "https://graph.microsoft.com/v1.0";
const DEFAULT_AUTH_BASE: &str = "https://login.microsoftonline.com";

struct Component;

impl bindings::exports::greentic::component::descriptor::Guest for Component {
    fn describe() -> Vec<u8> {
        canonical_cbor_bytes(&build_describe_payload())
    }
}

impl bindings::exports::greentic::component::runtime::Guest for Component {
    fn invoke(op: String, input_cbor: Vec<u8>) -> Vec<u8> {
        cbor_json_invoke_bridge(&op, &input_cbor, Some("send"), |op, input| {
            dispatch_json_invoke(op, input)
        })
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
        use bindings::exports::greentic::component::qa::Mode;
        let mode_str = match mode {
            Mode::Default => "default",
            Mode::Setup => "setup",
            Mode::Upgrade => "upgrade",
            Mode::Remove => "remove",
        };
        apply_answers_impl(mode_str, answers_cbor)
    }
}

impl bindings::exports::greentic::component::component_i18n::Guest for Component {
    fn i18n_keys() -> Vec<String> {
        provider_common::helpers::i18n_keys_from(I18N_KEYS)
    }

    fn i18n_bundle(locale: String) -> Vec<u8> {
        describe::i18n_bundle(locale)
    }
}

// Backward-compatible schema-core-api export for operator v0.4.x
impl bindings::exports::greentic::provider_schema_core::schema_core_api::Guest for Component {
    fn describe() -> Vec<u8> {
        schema_core_describe(&build_describe_payload())
    }

    fn validate_config(_config_json: Vec<u8>) -> Vec<u8> {
        schema_core_validate_config()
    }

    fn healthcheck() -> Vec<u8> {
        schema_core_healthcheck()
    }

    fn invoke(op: String, input_json: Vec<u8>) -> Vec<u8> {
        if let Some(result) = provider_common::qa_invoke_bridge::dispatch_qa_ops(
            &op,
            &input_json,
            "teams",
            describe::SETUP_QUESTIONS,
            describe::DEFAULT_KEYS,
            I18N_KEYS,
            apply_answers_impl,
        ) {
            return result;
        }
        let op = if op == "run" { "send".to_string() } else { op };
        dispatch_json_invoke(&op, &input_json)
    }
}

bindings::export!(Component with_types_in bindings);

fn apply_answers_impl(mode: &str, answers_cbor: Vec<u8>) -> Vec<u8> {
    let answers: Value = match decode_cbor(&answers_cbor) {
        Ok(value) => value,
        Err(err) => {
            return canonical_cbor_bytes(&ApplyAnswersResult::<ProviderConfigOut>::decode_error(
                format!("invalid answers cbor: {err}"),
            ));
        }
    };

    if mode == "remove" {
        return canonical_cbor_bytes(&ApplyAnswersResult::<ProviderConfigOut>::remove_default());
    }

    let mut merged = existing_config_from_answers(&answers).unwrap_or_else(default_config_out);
    let answer_obj = answers.as_object();
    let has = |key: &str| answer_obj.is_some_and(|obj| obj.contains_key(key));

    if mode == "setup" || mode == "default" {
        merged.enabled = answers
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(merged.enabled);
        merged.tenant_id = string_or_default(&answers, "tenant_id", &merged.tenant_id);
        merged.client_id = string_or_default(&answers, "client_id", &merged.client_id);
        merged.public_base_url =
            string_or_default(&answers, "public_base_url", &merged.public_base_url);
        merged.team_id = optional_string_from(&answers, "team_id").or(merged.team_id.clone());
        merged.channel_id =
            optional_string_from(&answers, "channel_id").or(merged.channel_id.clone());
        merged.graph_base_url =
            string_or_default(&answers, "graph_base_url", &merged.graph_base_url);
        merged.auth_base_url = string_or_default(&answers, "auth_base_url", &merged.auth_base_url);
        merged.token_scope = string_or_default(&answers, "token_scope", &merged.token_scope);
        merged.client_secret =
            optional_string_from(&answers, "client_secret").or(merged.client_secret.clone());
        merged.refresh_token =
            optional_string_from(&answers, "refresh_token").or(merged.refresh_token.clone());
    }

    if mode == "upgrade" {
        if has("enabled") {
            merged.enabled = answers
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(merged.enabled);
        }
        if has("tenant_id") {
            merged.tenant_id = string_or_default(&answers, "tenant_id", &merged.tenant_id);
        }
        if has("client_id") {
            merged.client_id = string_or_default(&answers, "client_id", &merged.client_id);
        }
        if has("public_base_url") {
            merged.public_base_url =
                string_or_default(&answers, "public_base_url", &merged.public_base_url);
        }
        if has("team_id") {
            merged.team_id = optional_string_from(&answers, "team_id");
        }
        if has("channel_id") {
            merged.channel_id = optional_string_from(&answers, "channel_id");
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
        if has("client_secret") {
            merged.client_secret = optional_string_from(&answers, "client_secret");
        }
        if has("refresh_token") {
            merged.refresh_token = optional_string_from(&answers, "refresh_token");
        }
    }

    if let Err(error) = validate_config_out(&merged) {
        return canonical_cbor_bytes(&ApplyAnswersResult::<ProviderConfigOut>::validation_error(
            error,
        ));
    }

    canonical_cbor_bytes(&ApplyAnswersResult::success(merged))
}

fn dispatch_json_invoke(op: &str, input_json: &[u8]) -> Vec<u8> {
    match op {
        "send" => handle_send(input_json),
        "reply" => handle_reply(input_json),
        "ingest_http" => ingest_http(input_json),
        "render_plan" => render_plan(input_json),
        "encode" => encode_op(input_json),
        "send_payload" => send_payload(input_json),
        "subscription_ensure" => subscription_ensure(input_json),
        "subscription_renew" => subscription_renew(input_json),
        "subscription_delete" => subscription_delete(input_json),
        other => json_bytes(&json!({"ok": false, "error": format!("unsupported op: {other}")})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use config::parse_config_bytes;

    #[test]
    fn parse_config_requires_new_fields() {
        let cfg = br#"{"enabled":true,"tenant_id":"t","client_id":"c","public_base_url":"https://example.com","graph_base_url":"https://graph.microsoft.com/v1.0","auth_base_url":"https://login.microsoftonline.com","token_scope":"https://graph.microsoft.com/.default"}"#;
        let parsed = parse_config_bytes(cfg).expect("valid config");
        assert!(parsed.enabled);
    }

    #[test]
    fn load_config_prefers_nested() {
        let input = json!({
            "config": {
                "enabled": true,
                "tenant_id": "t",
                "client_id": "c",
                "public_base_url": "https://example.com",
                "graph_base_url": "https://graph.microsoft.com/v1.0",
                "auth_base_url": "https://login.microsoftonline.com",
                "token_scope": "https://graph.microsoft.com/.default"
            },
            "tenant_id": "outer"
        });
        let cfg = config::load_config(&input).expect("cfg");
        assert_eq!(cfg.tenant_id, "t");
    }

    #[test]
    fn parse_config_rejects_unknown() {
        let cfg = br#"{"enabled":true,"tenant_id":"t","client_id":"c","public_base_url":"https://example.com","graph_base_url":"https://graph.microsoft.com/v1.0","auth_base_url":"https://login.microsoftonline.com","token_scope":"https://graph.microsoft.com/.default","unexpected":true}"#;
        let err = parse_config_bytes(cfg).unwrap_err();
        assert!(err.contains("unknown field"));
    }

    provider_common::standard_provider_tests! {
        describe_fn: build_describe_payload,
        qa_spec_fn: build_qa_spec,
        i18n_keys: I18N_KEYS,
        world_id: WORLD_ID,
        provider_id: PROVIDER_ID,
        schema_hash: "6eeefd5235cda241a0c38d9748f6a224779e6db1b73b2cd9947ef52a23d8462d",
        qa_default_keys: ["tenant_id", "client_id", "public_base_url"],
        mode_type: bindings::exports::greentic::component::qa::Mode,
        component_type: Component,
        qa_guest_path: bindings::exports::greentic::component::qa::Guest,
        validation_answers: {"tenant_id": "tenant-a", "client_id": "client-a", "public_base_url": "not-a-url"},
        validation_field: "public_base_url",
    }

    #[test]
    fn apply_answers_upgrade_preserves_unspecified_fields() {
        use bindings::exports::greentic::component::qa::Guest as QaGuest;
        use bindings::exports::greentic::component::qa::Mode;
        let answers = json!({
            "existing_config": {
                "enabled": true,
                "tenant_id": "tenant-a",
                "client_id": "client-a",
                "public_base_url": "https://example.com",
                "team_id": "team-a",
                "channel_id": "channel-a",
                "graph_base_url": "https://graph.microsoft.com/v1.0",
                "auth_base_url": "https://login.microsoftonline.com",
                "token_scope": "scope-a",
                "client_secret": "secret-a",
                "refresh_token": "refresh-a"
            },
            "channel_id": "channel-b"
        });
        let out =
            <Component as QaGuest>::apply_answers(Mode::Upgrade, canonical_cbor_bytes(&answers));
        let out_json: Value = decode_cbor(&out).expect("decode apply output");
        assert_eq!(out_json.get("ok"), Some(&Value::Bool(true)));
        let config = out_json.get("config").expect("config object");
        assert_eq!(
            config.get("tenant_id"),
            Some(&Value::String("tenant-a".to_string()))
        );
        assert_eq!(
            config.get("channel_id"),
            Some(&Value::String("channel-b".to_string()))
        );
    }
}
