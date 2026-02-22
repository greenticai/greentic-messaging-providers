use provider_common::component_v0_6::{canonical_cbor_bytes, decode_cbor};
use provider_common::helpers::{
    cbor_json_invoke_bridge, existing_config_from_answers, json_bytes, optional_string_from,
    schema_core_describe, schema_core_healthcheck, schema_core_validate_config, string_or_default,
};
use provider_common::qa_helpers::ApplyAnswersResult;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

mod bindings {
    wit_bindgen::generate!({
        path: "wit/messaging-provider-webex",
        world: "component-v0-v6-v0",
        generate_all
    });
}

mod config;
mod describe;
mod ops;

const PROVIDER_ID: &str = "messaging-provider-webex";
const PROVIDER_TYPE: &str = "messaging.webex.bot";
const WORLD_ID: &str = "component-v0-v6-v0";
const DEFAULT_API_BASE: &str = "https://webexapis.com/v1";
const DEFAULT_TOKEN_KEY: &str = "WEBEX_BOT_TOKEN";

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct ProviderConfig {
    #[serde(default = "config::default_enabled")]
    enabled: bool,
    public_base_url: String,
    #[serde(default)]
    default_room_id: Option<String>,
    #[serde(default)]
    default_to_person_email: Option<String>,
    #[serde(default)]
    api_base_url: Option<String>,
    #[serde(default)]
    bot_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProviderConfigOut {
    enabled: bool,
    public_base_url: String,
    default_room_id: Option<String>,
    default_to_person_email: Option<String>,
    api_base_url: String,
    bot_token: Option<String>,
}

struct Component;

impl bindings::exports::greentic::component::descriptor::Guest for Component {
    fn describe() -> Vec<u8> {
        canonical_cbor_bytes(&describe::build_describe_payload())
    }
}

impl bindings::exports::greentic::component::runtime::Guest for Component {
    fn invoke(op: String, input_cbor: Vec<u8>) -> Vec<u8> {
        cbor_json_invoke_bridge(&op, &input_cbor, Some("send"), dispatch_json_invoke)
    }
}

impl bindings::exports::greentic::component::qa::Guest for Component {
    fn qa_spec(mode: bindings::exports::greentic::component::qa::Mode) -> Vec<u8> {
        canonical_cbor_bytes(&describe::build_qa_spec(mode))
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
        provider_common::helpers::i18n_keys_from(describe::I18N_KEYS)
    }

    fn i18n_bundle(locale: String) -> Vec<u8> {
        describe::i18n_bundle(locale)
    }
}

// Backward-compatible schema-core-api export for operator v0.4.x
impl bindings::exports::greentic::provider_schema_core::schema_core_api::Guest for Component {
    fn describe() -> Vec<u8> {
        schema_core_describe(&describe::build_describe_payload())
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
            "webex",
            describe::SETUP_QUESTIONS,
            describe::DEFAULT_KEYS,
            describe::I18N_KEYS,
            apply_answers_impl,
        ) {
            return result;
        }
        let op = if op == "run" { "send" } else { op.as_str() };
        dispatch_json_invoke(op, &input_json)
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

    let mut merged =
        existing_config_from_answers(&answers).unwrap_or_else(config::default_config_out);
    let answer_obj = answers.as_object();
    let has = |key: &str| answer_obj.is_some_and(|obj| obj.contains_key(key));

    if mode == "setup" || mode == "default" {
        merged.enabled = answers
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(merged.enabled);
        merged.public_base_url =
            string_or_default(&answers, "public_base_url", &merged.public_base_url);
        merged.default_room_id =
            optional_string_from(&answers, "default_room_id").or(merged.default_room_id.clone());
        merged.default_to_person_email = optional_string_from(&answers, "default_to_person_email")
            .or(merged.default_to_person_email.clone());
        merged.api_base_url = string_or_default(&answers, "api_base_url", &merged.api_base_url);
        if merged.api_base_url.trim().is_empty() {
            merged.api_base_url = DEFAULT_API_BASE.to_string();
        }
        merged.bot_token = optional_string_from(&answers, "bot_token").or(merged.bot_token.clone());
    }

    if mode == "upgrade" {
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
        if has("default_room_id") {
            merged.default_room_id = optional_string_from(&answers, "default_room_id");
        }
        if has("default_to_person_email") {
            merged.default_to_person_email =
                optional_string_from(&answers, "default_to_person_email");
        }
        if has("api_base_url") {
            merged.api_base_url = string_or_default(&answers, "api_base_url", &merged.api_base_url);
        }
        if has("bot_token") {
            merged.bot_token = optional_string_from(&answers, "bot_token");
        }
        if merged.api_base_url.trim().is_empty() {
            merged.api_base_url = DEFAULT_API_BASE.to_string();
        }
    }

    if let Err(error) = config::validate_config_out(&merged) {
        return canonical_cbor_bytes(&ApplyAnswersResult::<ProviderConfigOut>::validation_error(
            error,
        ));
    }

    canonical_cbor_bytes(&ApplyAnswersResult::success(merged))
}

fn dispatch_json_invoke(op: &str, input_json: &[u8]) -> Vec<u8> {
    match op {
        "send" => ops::handle_send(input_json),
        "reply" => ops::handle_reply(input_json),
        "ingest_http" => ops::ingest_http(input_json),
        "render_plan" => ops::render_plan(input_json),
        "encode" => ops::encode_op(input_json),
        "send_payload" => ops::send_payload(input_json),
        other => json_bytes(&json!({"ok": false, "error": format!("unsupported op: {other}")})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_webex_body_includes_markdown_and_attachment() {
        let card = json!({
            "$schema": "http://adaptivecards.io/schemas/adaptive-card.json",
            "type": "AdaptiveCard",
            "version": "1.3",
            "body": [{"type": "TextBlock", "text": "hi"}]
        });
        let mut body = ops::build_webex_body(Some(&card), None, " ");
        body.insert("toPersonEmail".into(), Value::String("example@test".into()));
        assert_eq!(body.get("markdown"), Some(&Value::String(" ".into())));
        assert_eq!(
            body.get("toPersonEmail"),
            Some(&Value::String("example@test".into()))
        );
        let attachments = body
            .get("attachments")
            .and_then(Value::as_array)
            .expect("attachments present");
        assert_eq!(
            attachments[0]
                .get("contentType")
                .and_then(Value::as_str)
                .unwrap(),
            "application/vnd.microsoft.card.adaptive"
        );
        assert!(attachments[0].get("content").is_some());
    }

    #[test]
    fn format_webex_error_includes_body_text_when_present() {
        let msg = ops::format_webex_error(400, br#"{"message":"bad request"}"#);
        assert!(msg.contains("webex returned status 400"));
        assert!(msg.contains(r#"{"message":"bad request"}"#));
        let empty = ops::format_webex_error(500, b"");
        assert_eq!(empty, "webex returned status 500");
    }

    #[test]
    fn parse_config_requires_new_fields() {
        let cfg = br#"{"enabled":true,"public_base_url":"https://example.com","api_base_url":"https://webexapis.com/v1"}"#;
        let parsed = config::parse_config_bytes(cfg).expect("valid config");
        assert!(parsed.enabled);
    }

    #[test]
    fn load_config_defaults_to_token_key() {
        let input = json!({});
        let cfg = config::load_config(&input).unwrap();
        assert!(cfg.default_room_id.is_none());
    }

    #[test]
    fn parse_config_rejects_unknown() {
        let cfg = br#"{"enabled":true,"public_base_url":"https://example.com","api_base_url":"https://webexapis.com/v1","default_room_id":"k","unexpected":true}"#;
        let err = config::parse_config_bytes(cfg).unwrap_err();
        assert!(err.contains("unknown field"));
    }

    #[test]
    fn apply_answers_upgrade_preserves_unspecified_fields() {
        use bindings::exports::greentic::component::qa::Guest as QaGuest;
        use bindings::exports::greentic::component::qa::Mode;
        let answers = json!({
            "existing_config": {
                "enabled": true,
                "public_base_url": "https://example.com",
                "default_room_id": "room-a",
                "default_to_person_email": "a@example.com",
                "api_base_url": "https://webexapis.com/v1",
                "bot_token": "token-a"
            },
            "default_room_id": "room-b"
        });
        let out =
            <Component as QaGuest>::apply_answers(Mode::Upgrade, canonical_cbor_bytes(&answers));
        let out_json: Value = decode_cbor(&out).expect("decode apply output");
        assert_eq!(out_json.get("ok"), Some(&Value::Bool(true)));
        let config = out_json.get("config").expect("config object");
        assert_eq!(
            config.get("public_base_url"),
            Some(&Value::String("https://example.com".to_string()))
        );
        assert_eq!(
            config.get("bot_token"),
            Some(&Value::String("token-a".to_string()))
        );
        assert_eq!(
            config.get("default_room_id"),
            Some(&Value::String("room-b".to_string()))
        );
    }

    provider_common::standard_provider_tests! {
        describe_fn: describe::build_describe_payload,
        qa_spec_fn: describe::build_qa_spec,
        i18n_keys: describe::I18N_KEYS,
        world_id: WORLD_ID,
        provider_id: PROVIDER_ID,
        schema_hash: "074aca486987c019467084e02a4c5ace102a333f7755bb0e01da3620bcb8ae85",
        qa_default_keys: ["public_base_url"],
        mode_type: bindings::exports::greentic::component::qa::Mode,
        component_type: Component,
        qa_guest_path: bindings::exports::greentic::component::qa::Guest,
        validation_answers: {"public_base_url": "not-a-url"},
        validation_field: "public_base_url",
    }
}
