use provider_common::component_v0_6::{
    DescribePayload, QaSpec, RedactionRule, SchemaIr, schema_hash,
};
use provider_common::helpers::{
    op, schema_bool_ir, schema_obj, schema_secret, schema_str, schema_str_fmt,
};

use crate::{PROVIDER_ID, WORLD_ID};

pub(crate) const I18N_KEYS: &[&str] = &[
    "telegram.op.run.title",
    "telegram.op.run.description",
    "telegram.op.send.title",
    "telegram.op.send.description",
    "telegram.op.reply.title",
    "telegram.op.reply.description",
    "telegram.op.ingest_http.title",
    "telegram.op.ingest_http.description",
    "telegram.op.render_plan.title",
    "telegram.op.render_plan.description",
    "telegram.op.encode.title",
    "telegram.op.encode.description",
    "telegram.op.send_payload.title",
    "telegram.op.send_payload.description",
    "telegram.schema.input.title",
    "telegram.schema.input.description",
    "telegram.schema.input.message.title",
    "telegram.schema.input.message.description",
    "telegram.schema.output.title",
    "telegram.schema.output.description",
    "telegram.schema.output.ok.title",
    "telegram.schema.output.ok.description",
    "telegram.schema.output.message_id.title",
    "telegram.schema.output.message_id.description",
    "telegram.schema.config.title",
    "telegram.schema.config.description",
    "telegram.schema.config.enabled.title",
    "telegram.schema.config.enabled.description",
    "telegram.schema.config.public_base_url.title",
    "telegram.schema.config.public_base_url.description",
    "telegram.schema.config.default_chat_id.title",
    "telegram.schema.config.default_chat_id.description",
    "telegram.schema.config.api_base_url.title",
    "telegram.schema.config.api_base_url.description",
    "telegram.schema.config.bot_token.title",
    "telegram.schema.config.bot_token.description",
    "telegram.qa.default.title",
    "telegram.qa.setup.title",
    "telegram.qa.upgrade.title",
    "telegram.qa.remove.title",
    "telegram.qa.setup.enabled",
    "telegram.qa.setup.public_base_url",
    "telegram.qa.setup.default_chat_id",
    "telegram.qa.setup.api_base_url",
    "telegram.qa.setup.bot_token",
];

pub(crate) const SETUP_QUESTIONS: &[provider_common::helpers::QaQuestionDef] = &[
    ("enabled", "telegram.qa.setup.enabled", true),
    ("public_base_url", "telegram.qa.setup.public_base_url", true),
    (
        "default_chat_id",
        "telegram.qa.setup.default_chat_id",
        false,
    ),
    ("api_base_url", "telegram.qa.setup.api_base_url", true),
    ("bot_token", "telegram.qa.setup.bot_token", false),
];

pub(crate) const DEFAULT_KEYS: &[&str] = &["public_base_url"];

pub(crate) const I18N_PAIRS: &[(&str, &str)] = &[
    ("telegram.op.run.title", "Run"),
    (
        "telegram.op.run.description",
        "Run Telegram provider operation",
    ),
    ("telegram.op.send.title", "Send"),
    ("telegram.op.send.description", "Send a Telegram message"),
    ("telegram.op.reply.title", "Reply"),
    (
        "telegram.op.reply.description",
        "Reply to a Telegram message",
    ),
    ("telegram.op.ingest_http.title", "Ingest HTTP"),
    (
        "telegram.op.ingest_http.description",
        "Normalize Telegram webhook payload",
    ),
    ("telegram.op.render_plan.title", "Render Plan"),
    (
        "telegram.op.render_plan.description",
        "Render universal message plan",
    ),
    ("telegram.op.encode.title", "Encode"),
    (
        "telegram.op.encode.description",
        "Encode universal payload for Telegram",
    ),
    ("telegram.op.send_payload.title", "Send Payload"),
    (
        "telegram.op.send_payload.description",
        "Send encoded payload to Telegram API",
    ),
    ("telegram.schema.input.title", "Telegram input"),
    (
        "telegram.schema.input.description",
        "Input for Telegram run/send operations",
    ),
    ("telegram.schema.input.message.title", "Message"),
    ("telegram.schema.input.message.description", "Message text"),
    ("telegram.schema.output.title", "Telegram output"),
    (
        "telegram.schema.output.description",
        "Result of Telegram operation",
    ),
    ("telegram.schema.output.ok.title", "Success"),
    (
        "telegram.schema.output.ok.description",
        "Whether operation succeeded",
    ),
    ("telegram.schema.output.message_id.title", "Message ID"),
    (
        "telegram.schema.output.message_id.description",
        "Telegram message identifier",
    ),
    ("telegram.schema.config.title", "Telegram config"),
    (
        "telegram.schema.config.description",
        "Telegram provider configuration",
    ),
    ("telegram.schema.config.enabled.title", "Enabled"),
    (
        "telegram.schema.config.enabled.description",
        "Enable this provider",
    ),
    (
        "telegram.schema.config.public_base_url.title",
        "Public base URL",
    ),
    (
        "telegram.schema.config.public_base_url.description",
        "Public URL for webhook callbacks",
    ),
    (
        "telegram.schema.config.default_chat_id.title",
        "Default chat ID",
    ),
    (
        "telegram.schema.config.default_chat_id.description",
        "Chat ID used when destination is omitted",
    ),
    ("telegram.schema.config.api_base_url.title", "API base URL"),
    (
        "telegram.schema.config.api_base_url.description",
        "Telegram Bot API base URL",
    ),
    ("telegram.schema.config.bot_token.title", "Bot token"),
    (
        "telegram.schema.config.bot_token.description",
        "Bot token for Telegram API calls",
    ),
    ("telegram.qa.default.title", "Default"),
    ("telegram.qa.setup.title", "Setup"),
    ("telegram.qa.upgrade.title", "Upgrade"),
    ("telegram.qa.remove.title", "Remove"),
    ("telegram.qa.setup.enabled", "Enable provider"),
    ("telegram.qa.setup.public_base_url", "Public base URL"),
    ("telegram.qa.setup.default_chat_id", "Default chat ID"),
    ("telegram.qa.setup.api_base_url", "API base URL"),
    ("telegram.qa.setup.bot_token", "Bot token"),
];

pub(crate) fn build_describe_payload() -> DescribePayload {
    let input_schema = input_schema();
    let output_schema = output_schema();
    let config_schema = config_schema();
    DescribePayload {
        provider: PROVIDER_ID.to_string(),
        world: WORLD_ID.to_string(),
        operations: vec![
            op(
                "run",
                "telegram.op.run.title",
                "telegram.op.run.description",
            ),
            op(
                "send",
                "telegram.op.send.title",
                "telegram.op.send.description",
            ),
            op(
                "reply",
                "telegram.op.reply.title",
                "telegram.op.reply.description",
            ),
            op(
                "ingest_http",
                "telegram.op.ingest_http.title",
                "telegram.op.ingest_http.description",
            ),
            op(
                "render_plan",
                "telegram.op.render_plan.title",
                "telegram.op.render_plan.description",
            ),
            op(
                "encode",
                "telegram.op.encode.title",
                "telegram.op.encode.description",
            ),
            op(
                "send_payload",
                "telegram.op.send_payload.title",
                "telegram.op.send_payload.description",
            ),
        ],
        input_schema: input_schema.clone(),
        output_schema: output_schema.clone(),
        config_schema: config_schema.clone(),
        redactions: vec![RedactionRule {
            path: "$.bot_token".to_string(),
            strategy: "replace".to_string(),
        }],
        schema_hash: schema_hash(&input_schema, &output_schema, &config_schema),
    }
}

pub(crate) fn build_qa_spec(
    mode: crate::bindings::exports::greentic::component::qa::Mode,
) -> QaSpec {
    use crate::bindings::exports::greentic::component::qa::Mode;
    let mode_str = match mode {
        Mode::Default => "default",
        Mode::Setup => "setup",
        Mode::Upgrade => "upgrade",
        Mode::Remove => "remove",
    };
    provider_common::helpers::qa_spec_for_mode(mode_str, "telegram", SETUP_QUESTIONS, DEFAULT_KEYS)
}

fn input_schema() -> SchemaIr {
    schema_obj(
        "telegram.schema.input.title",
        "telegram.schema.input.description",
        vec![(
            "message",
            true,
            schema_str(
                "telegram.schema.input.message.title",
                "telegram.schema.input.message.description",
            ),
        )],
        true,
    )
}

fn output_schema() -> SchemaIr {
    schema_obj(
        "telegram.schema.output.title",
        "telegram.schema.output.description",
        vec![
            (
                "ok",
                true,
                schema_bool_ir(
                    "telegram.schema.output.ok.title",
                    "telegram.schema.output.ok.description",
                ),
            ),
            (
                "message_id",
                false,
                schema_str(
                    "telegram.schema.output.message_id.title",
                    "telegram.schema.output.message_id.description",
                ),
            ),
        ],
        true,
    )
}

fn config_schema() -> SchemaIr {
    schema_obj(
        "telegram.schema.config.title",
        "telegram.schema.config.description",
        vec![
            (
                "enabled",
                true,
                schema_bool_ir(
                    "telegram.schema.config.enabled.title",
                    "telegram.schema.config.enabled.description",
                ),
            ),
            (
                "public_base_url",
                true,
                schema_str_fmt(
                    "telegram.schema.config.public_base_url.title",
                    "telegram.schema.config.public_base_url.description",
                    "uri",
                ),
            ),
            (
                "default_chat_id",
                false,
                schema_str(
                    "telegram.schema.config.default_chat_id.title",
                    "telegram.schema.config.default_chat_id.description",
                ),
            ),
            (
                "api_base_url",
                true,
                schema_str_fmt(
                    "telegram.schema.config.api_base_url.title",
                    "telegram.schema.config.api_base_url.description",
                    "uri",
                ),
            ),
            (
                "bot_token",
                false,
                schema_secret(
                    "telegram.schema.config.bot_token.title",
                    "telegram.schema.config.bot_token.description",
                ),
            ),
        ],
        false,
    )
}
