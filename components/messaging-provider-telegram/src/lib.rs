use provider_common::component_v0_6::{canonical_cbor_bytes, decode_cbor};
use provider_common::helpers::{
    cbor_json_invoke_bridge, existing_config_from_answers, json_bytes, optional_string_from,
    schema_core_describe, schema_core_healthcheck, schema_core_validate_config, string_or_default,
};
use provider_common::qa_helpers::ApplyAnswersResult;
use serde_json::{Value, json};

mod bindings {
    wit_bindgen::generate!({
        path: "wit/messaging-provider-telegram",
        world: "component-v0-v6-v0",
        generate_all
    });
}

mod config;
mod describe;
mod ops;

use config::{ProviderConfigOut, default_config_out, validate_config_out};
use describe::{
    DEFAULT_KEYS, I18N_KEYS, I18N_PAIRS, SETUP_QUESTIONS, build_describe_payload, build_qa_spec,
};
use ops::{encode_op, handle_reply, handle_send, ingest_http, render_plan, send_payload};

const PROVIDER_ID: &str = "messaging-provider-telegram";
const PROVIDER_TYPE: &str = "messaging.telegram.bot";
const WORLD_ID: &str = "component-v0-v6-v0";
const DEFAULT_API_BASE: &str = "https://api.telegram.org";
const TOKEN_SECRET: &str = "TELEGRAM_BOT_TOKEN";

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
        provider_common::helpers::i18n_bundle_from_pairs(locale, I18N_PAIRS)
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
            "telegram",
            SETUP_QUESTIONS,
            DEFAULT_KEYS,
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
        merged.public_base_url =
            string_or_default(&answers, "public_base_url", &merged.public_base_url);
        merged.default_chat_id =
            optional_string_from(&answers, "default_chat_id").or(merged.default_chat_id.clone());
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
        if has("default_chat_id") {
            merged.default_chat_id = optional_string_from(&answers, "default_chat_id");
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
        other => json_bytes(&json!({"ok": false, "error": format!("unsupported op: {other}")})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use greentic_types::messaging::universal_dto::{EncodeInV1, RenderPlanInV1};
    use greentic_types::{
        Attachment, ChannelMessageEnvelope, Destination, EnvId, MessageMetadata, TenantCtx,
        TenantId,
    };
    use provider_common::component_v0_6::{canonical_cbor_bytes, decode_cbor};

    #[test]
    fn load_config_prefers_nested_config() {
        let input = json!({
            "config": {
                "enabled": true,
                "public_base_url": "https://example.com",
                "default_chat_id": "abc",
                "api_base_url": "https://api.telegram.org"
            },
        });
        let cfg = config::load_config(&input).expect("config");
        assert_eq!(cfg.default_chat_id.as_deref(), Some("abc"));
    }

    #[test]
    fn parse_config_requires_new_fields() {
        let cfg = br#"{"enabled":true,"public_base_url":"https://example.com","api_base_url":"https://api.telegram.org"}"#;
        let parsed = config::parse_config_bytes(cfg).expect("config");
        assert!(parsed.enabled);
    }

    #[test]
    fn parse_config_bytes_rejects_unknown_fields() {
        let cfg = br#"{ "enabled": true, "public_base_url": "https://example.com", "api_base_url": "https://api.telegram.org", "default_chat_id": "abc", "unknown": "field" }"#;
        let err = config::parse_config_bytes(cfg).expect_err("should fail");
        assert!(err.contains("unknown field"));
    }

    #[test]
    fn extract_ids_handles_strings() {
        let body = json!({"result": {"message_id": "42"}});
        let (id, provider) = ops::extract_ids(&body);
        assert_eq!(id, "42");
        assert_eq!(provider, "tg:42");
    }

    provider_common::standard_provider_tests! {
        describe_fn: build_describe_payload,
        qa_spec_fn: build_qa_spec,
        i18n_keys: I18N_KEYS,
        world_id: WORLD_ID,
        provider_id: PROVIDER_ID,
        schema_hash: "be8773298b0229af6f641e622417c198970df42bac96cc560dd44569c4034328",
        qa_default_keys: ["public_base_url"],
        mode_type: bindings::exports::greentic::component::qa::Mode,
        component_type: Component,
        qa_guest_path: bindings::exports::greentic::component::qa::Guest,
        validation_answers: {"public_base_url": "not-a-url"},
        validation_field: "public_base_url",
    }

    #[test]
    fn apply_answers_upgrade_preserves_unspecified_fields() {
        use bindings::exports::greentic::component::qa::Guest as QaGuest;
        use bindings::exports::greentic::component::qa::Mode;
        let answers = json!({
            "existing_config": {
                "enabled": true,
                "public_base_url": "https://example.com",
                "default_chat_id": "123",
                "api_base_url": "https://api.telegram.org",
                "bot_token": "token-a"
            },
            "default_chat_id": "456"
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
            config.get("default_chat_id"),
            Some(&Value::String("456".to_string()))
        );
    }

    // ── Media encode helpers & tests ──────────────────────────────────────

    fn make_encode_input(
        text: Option<&str>,
        metadata: &[(&str, &str)],
        attachments: Vec<Attachment>,
    ) -> Vec<u8> {
        let env = EnvId::try_from("test").unwrap();
        let tenant = TenantId::try_from("test").unwrap();
        let mut meta = MessageMetadata::new();
        for (k, v) in metadata {
            meta.insert(k.to_string(), v.to_string());
        }
        let envelope = ChannelMessageEnvelope {
            id: "test-msg".to_string(),
            tenant: TenantCtx::new(env, tenant),
            channel: "telegram".to_string(),
            session_id: "test-session".to_string(),
            reply_scope: None,
            from: None,
            to: vec![Destination {
                id: "123456".to_string(),
                kind: Some("chat".into()),
            }],
            correlation_id: None,
            text: text.map(String::from),
            attachments,
            metadata: meta,
        };
        let encode_in = EncodeInV1 {
            message: envelope,
            plan: RenderPlanInV1 {
                message: ChannelMessageEnvelope {
                    id: String::new(),
                    tenant: TenantCtx::new(
                        EnvId::try_from("test").unwrap(),
                        TenantId::try_from("test").unwrap(),
                    ),
                    channel: String::new(),
                    session_id: String::new(),
                    reply_scope: None,
                    from: None,
                    to: vec![],
                    correlation_id: None,
                    text: None,
                    attachments: vec![],
                    metadata: MessageMetadata::new(),
                },
                metadata: std::collections::BTreeMap::new(),
            },
        };
        serde_json::to_vec(&encode_in).unwrap()
    }

    fn decode_encode_envelope(result: &[u8]) -> ChannelMessageEnvelope {
        let val: Value = serde_json::from_slice(result).expect("parse encode result");
        assert_eq!(
            val.get("ok"),
            Some(&Value::Bool(true)),
            "encode should succeed: {val}"
        );
        let payload = val.get("payload").expect("payload field");
        let body_b64 = payload
            .get("body_b64")
            .and_then(Value::as_str)
            .expect("body_b64");
        let body_bytes = base64::engine::general_purpose::STANDARD
            .decode(body_b64)
            .expect("decode base64");
        serde_json::from_slice(&body_bytes).expect("parse envelope from payload")
    }

    #[test]
    fn encode_op_video_metadata() {
        let input = make_encode_input(
            Some("hello"),
            &[("tg_video_url", "https://example.com/video.mp4")],
            vec![],
        );
        let env = decode_encode_envelope(&encode_op(&input));
        assert_eq!(
            env.metadata.get("tg_video").map(String::as_str),
            Some("https://example.com/video.mp4")
        );
    }

    #[test]
    fn encode_op_audio_metadata() {
        let input = make_encode_input(
            Some("hello"),
            &[
                ("tg_audio_url", "https://example.com/audio.mp3"),
                ("tg_audio_caption", "My Song"),
            ],
            vec![],
        );
        let env = decode_encode_envelope(&encode_op(&input));
        assert_eq!(
            env.metadata.get("tg_audio").map(String::as_str),
            Some("https://example.com/audio.mp3")
        );
        assert_eq!(
            env.metadata.get("tg_audio_caption").map(String::as_str),
            Some("My Song")
        );
    }

    #[test]
    fn encode_op_sticker_metadata() {
        let input = make_encode_input(
            Some("hello"),
            &[("tg_sticker_url", "https://example.com/sticker.webp")],
            vec![],
        );
        let env = decode_encode_envelope(&encode_op(&input));
        assert_eq!(
            env.metadata.get("tg_sticker").map(String::as_str),
            Some("https://example.com/sticker.webp")
        );
    }

    #[test]
    fn encode_op_location_metadata() {
        let input = make_encode_input(
            Some("hello"),
            &[
                ("tg_location_latitude", "51.5074"),
                ("tg_location_longitude", "-0.1278"),
                ("tg_location_name", "London"),
                ("tg_location_address", "Westminster"),
            ],
            vec![],
        );
        let env = decode_encode_envelope(&encode_op(&input));
        let loc_str = env.metadata.get("tg_location").expect("tg_location");
        let loc: Value = serde_json::from_str(loc_str).expect("parse location JSON");
        assert_eq!(loc.get("latitude").and_then(Value::as_str), Some("51.5074"));
        assert_eq!(
            loc.get("longitude").and_then(Value::as_str),
            Some("-0.1278")
        );
        assert_eq!(loc.get("name").and_then(Value::as_str), Some("London"));
        assert_eq!(
            loc.get("address").and_then(Value::as_str),
            Some("Westminster")
        );
    }

    #[test]
    fn encode_op_attachment_video() {
        let input = make_encode_input(
            Some("hello"),
            &[],
            vec![Attachment {
                mime_type: "video/mp4".to_string(),
                url: "https://example.com/vid.mp4".to_string(),
                name: None,
                size_bytes: None,
            }],
        );
        let env = decode_encode_envelope(&encode_op(&input));
        assert_eq!(
            env.metadata.get("tg_video").map(String::as_str),
            Some("https://example.com/vid.mp4")
        );
    }

    #[test]
    fn encode_op_attachment_voice_ogg() {
        let input = make_encode_input(
            Some("hello"),
            &[],
            vec![Attachment {
                mime_type: "audio/ogg".to_string(),
                url: "https://example.com/voice.ogg".to_string(),
                name: None,
                size_bytes: None,
            }],
        );
        let env = decode_encode_envelope(&encode_op(&input));
        assert_eq!(
            env.metadata.get("tg_voice").map(String::as_str),
            Some("https://example.com/voice.ogg")
        );
    }

    #[test]
    fn encode_op_attachment_audio() {
        let input = make_encode_input(
            Some("hello"),
            &[],
            vec![Attachment {
                mime_type: "audio/mpeg".to_string(),
                url: "https://example.com/song.mp3".to_string(),
                name: None,
                size_bytes: None,
            }],
        );
        let env = decode_encode_envelope(&encode_op(&input));
        assert_eq!(
            env.metadata.get("tg_audio").map(String::as_str),
            Some("https://example.com/song.mp3")
        );
    }

    #[test]
    fn encode_op_attachment_sticker_webp() {
        let input = make_encode_input(
            Some("hello"),
            &[],
            vec![Attachment {
                mime_type: "image/webp".to_string(),
                url: "https://example.com/sticker.webp".to_string(),
                name: None,
                size_bytes: None,
            }],
        );
        let env = decode_encode_envelope(&encode_op(&input));
        assert_eq!(
            env.metadata.get("tg_sticker").map(String::as_str),
            Some("https://example.com/sticker.webp")
        );
    }

    #[test]
    fn encode_op_attachment_gif() {
        let input = make_encode_input(
            Some("hello"),
            &[],
            vec![Attachment {
                mime_type: "image/gif".to_string(),
                url: "https://example.com/anim.gif".to_string(),
                name: None,
                size_bytes: None,
            }],
        );
        let env = decode_encode_envelope(&encode_op(&input));
        assert_eq!(
            env.metadata.get("tg_animation").map(String::as_str),
            Some("https://example.com/anim.gif")
        );
    }

    #[test]
    fn encode_op_attachment_image() {
        let input = make_encode_input(
            Some("hello"),
            &[],
            vec![Attachment {
                mime_type: "image/jpeg".to_string(),
                url: "https://example.com/photo.jpg".to_string(),
                name: None,
                size_bytes: None,
            }],
        );
        let env = decode_encode_envelope(&encode_op(&input));
        assert_eq!(
            env.metadata.get("tg_photo").map(String::as_str),
            Some("https://example.com/photo.jpg")
        );
    }

    #[test]
    fn encode_op_attachment_document() {
        let input = make_encode_input(
            Some("hello"),
            &[],
            vec![Attachment {
                mime_type: "application/pdf".to_string(),
                url: "https://example.com/doc.pdf".to_string(),
                name: Some("report.pdf".to_string()),
                size_bytes: None,
            }],
        );
        let env = decode_encode_envelope(&encode_op(&input));
        assert_eq!(
            env.metadata.get("tg_document").map(String::as_str),
            Some("https://example.com/doc.pdf")
        );
    }

    #[test]
    fn encode_op_metadata_takes_precedence_over_attachments() {
        let input = make_encode_input(
            Some("hello"),
            &[("tg_video_url", "https://meta.com/video.mp4")],
            vec![Attachment {
                mime_type: "video/mp4".to_string(),
                url: "https://attach.com/video.mp4".to_string(),
                name: None,
                size_bytes: None,
            }],
        );
        let env = decode_encode_envelope(&encode_op(&input));
        assert_eq!(
            env.metadata.get("tg_video").map(String::as_str),
            Some("https://meta.com/video.mp4"),
            "metadata should take precedence over attachment"
        );
    }

    #[test]
    fn encode_op_strips_operator_double_quotes() {
        let input = make_encode_input(
            Some("hello"),
            &[("tg_video_url", "\"https://example.com/video.mp4\"")],
            vec![],
        );
        let env = decode_encode_envelope(&encode_op(&input));
        assert_eq!(
            env.metadata.get("tg_video").map(String::as_str),
            Some("https://example.com/video.mp4"),
            "operator double-quotes should be stripped"
        );
    }

    #[test]
    fn encode_op_backward_compat_ac_with_images() {
        let ac = json!({
            "type": "AdaptiveCard",
            "body": [
                {"type": "TextBlock", "text": "Hello AC"},
                {"type": "Image", "url": "https://example.com/img.png"}
            ]
        });
        let input = make_encode_input(
            Some("fallback text"),
            &[("adaptive_card", &ac.to_string())],
            vec![],
        );
        let env = decode_encode_envelope(&encode_op(&input));
        assert!(
            env.metadata.contains_key("ac_images"),
            "AC images should still be extracted"
        );
        let images: Vec<String> =
            serde_json::from_str(env.metadata.get("ac_images").unwrap()).unwrap();
        assert_eq!(images, vec!["https://example.com/img.png"]);
    }
}
