use provider_common::component_v0_6::{
    DescribePayload, QaSpec, RedactionRule, SchemaIr, schema_hash,
};
use provider_common::helpers::{
    i18n_bundle_from_pairs, op, schema_bool_ir, schema_obj, schema_secret, schema_str,
    schema_str_fmt,
};

use crate::{PROVIDER_ID, WORLD_ID};

pub(crate) const I18N_KEYS: &[&str] = &[
    "slack.op.run.title",
    "slack.op.run.description",
    "slack.op.send.title",
    "slack.op.send.description",
    "slack.op.reply.title",
    "slack.op.reply.description",
    "slack.op.ingest_http.title",
    "slack.op.ingest_http.description",
    "slack.op.render_plan.title",
    "slack.op.render_plan.description",
    "slack.op.encode.title",
    "slack.op.encode.description",
    "slack.op.send_payload.title",
    "slack.op.send_payload.description",
    "slack.schema.input.title",
    "slack.schema.input.description",
    "slack.schema.input.message.title",
    "slack.schema.input.message.description",
    "slack.schema.output.title",
    "slack.schema.output.description",
    "slack.schema.output.ok.title",
    "slack.schema.output.ok.description",
    "slack.schema.output.message_id.title",
    "slack.schema.output.message_id.description",
    "slack.schema.config.title",
    "slack.schema.config.description",
    "slack.schema.config.enabled.title",
    "slack.schema.config.enabled.description",
    "slack.schema.config.default_channel.title",
    "slack.schema.config.default_channel.description",
    "slack.schema.config.public_base_url.title",
    "slack.schema.config.public_base_url.description",
    "slack.schema.config.api_base_url.title",
    "slack.schema.config.api_base_url.description",
    "slack.schema.config.bot_token.title",
    "slack.schema.config.bot_token.description",
    "slack.qa.default.title",
    "slack.qa.setup.title",
    "slack.qa.upgrade.title",
    "slack.qa.remove.title",
    "slack.qa.setup.enabled",
    "slack.qa.setup.public_base_url",
    "slack.qa.setup.api_base_url",
    "slack.qa.setup.bot_token",
    "slack.qa.setup.default_channel",
];

pub(crate) const SETUP_QUESTIONS: &[provider_common::helpers::QaQuestionDef] = &[
    ("enabled", "slack.qa.setup.enabled", true),
    ("public_base_url", "slack.qa.setup.public_base_url", true),
    ("api_base_url", "slack.qa.setup.api_base_url", true),
    ("bot_token", "slack.qa.setup.bot_token", true),
    ("default_channel", "slack.qa.setup.default_channel", false),
];

pub(crate) const DEFAULT_KEYS: &[&str] = &["public_base_url", "bot_token"];

pub(crate) fn build_describe_payload() -> DescribePayload {
    let input_schema = input_schema();
    let output_schema = output_schema();
    let config_schema = config_schema();

    DescribePayload {
        provider: PROVIDER_ID.to_string(),
        world: WORLD_ID.to_string(),
        operations: vec![
            op("run", "slack.op.run.title", "slack.op.run.description"),
            op("send", "slack.op.send.title", "slack.op.send.description"),
            op(
                "reply",
                "slack.op.reply.title",
                "slack.op.reply.description",
            ),
            op(
                "ingest_http",
                "slack.op.ingest_http.title",
                "slack.op.ingest_http.description",
            ),
            op(
                "render_plan",
                "slack.op.render_plan.title",
                "slack.op.render_plan.description",
            ),
            op(
                "encode",
                "slack.op.encode.title",
                "slack.op.encode.description",
            ),
            op(
                "send_payload",
                "slack.op.send_payload.title",
                "slack.op.send_payload.description",
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
    provider_common::helpers::qa_spec_for_mode(mode_str, "slack", SETUP_QUESTIONS, DEFAULT_KEYS)
}

pub(crate) fn i18n_bundle(locale: String) -> Vec<u8> {
    i18n_bundle_from_pairs(
        locale,
        &[
            ("slack.op.run.title", "Run"),
            ("slack.op.run.description", "Run slack provider operation"),
            ("slack.op.send.title", "Send"),
            ("slack.op.send.description", "Send a Slack message"),
            ("slack.op.reply.title", "Reply"),
            ("slack.op.reply.description", "Reply in a Slack thread"),
            ("slack.op.ingest_http.title", "Ingest HTTP"),
            (
                "slack.op.ingest_http.description",
                "Normalize Slack webhook payload",
            ),
            ("slack.op.render_plan.title", "Render Plan"),
            (
                "slack.op.render_plan.description",
                "Render universal message plan",
            ),
            ("slack.op.encode.title", "Encode"),
            (
                "slack.op.encode.description",
                "Encode universal payload for Slack",
            ),
            ("slack.op.send_payload.title", "Send Payload"),
            (
                "slack.op.send_payload.description",
                "Send encoded payload to Slack API",
            ),
            ("slack.schema.input.title", "Slack input"),
            (
                "slack.schema.input.description",
                "Input for Slack run/send operations",
            ),
            ("slack.schema.input.message.title", "Message"),
            ("slack.schema.input.message.description", "Message text"),
            ("slack.schema.output.title", "Slack output"),
            (
                "slack.schema.output.description",
                "Result of Slack operation",
            ),
            ("slack.schema.output.ok.title", "Success"),
            (
                "slack.schema.output.ok.description",
                "Whether operation succeeded",
            ),
            ("slack.schema.output.message_id.title", "Message ID"),
            (
                "slack.schema.output.message_id.description",
                "Slack timestamp identifier",
            ),
            ("slack.schema.config.title", "Slack config"),
            (
                "slack.schema.config.description",
                "Slack provider configuration",
            ),
            ("slack.schema.config.enabled.title", "Enabled"),
            (
                "slack.schema.config.enabled.description",
                "Enable this provider",
            ),
            (
                "slack.schema.config.default_channel.title",
                "Default channel",
            ),
            (
                "slack.schema.config.default_channel.description",
                "Channel used when destination is omitted",
            ),
            (
                "slack.schema.config.public_base_url.title",
                "Public base URL",
            ),
            (
                "slack.schema.config.public_base_url.description",
                "Public URL for callbacks",
            ),
            ("slack.schema.config.api_base_url.title", "API base URL"),
            (
                "slack.schema.config.api_base_url.description",
                "Slack API base URL",
            ),
            ("slack.schema.config.bot_token.title", "Bot token"),
            (
                "slack.schema.config.bot_token.description",
                "Bot token for Slack API calls",
            ),
            ("slack.qa.default.title", "Default"),
            ("slack.qa.setup.title", "Setup"),
            ("slack.qa.upgrade.title", "Upgrade"),
            ("slack.qa.remove.title", "Remove"),
            ("slack.qa.setup.enabled", "Enable provider"),
            ("slack.qa.setup.public_base_url", "Public base URL"),
            ("slack.qa.setup.api_base_url", "API base URL"),
            ("slack.qa.setup.bot_token", "Bot token"),
            ("slack.qa.setup.default_channel", "Default channel"),
        ],
    )
}

fn input_schema() -> SchemaIr {
    schema_obj(
        "slack.schema.input.title",
        "slack.schema.input.description",
        vec![(
            "message",
            true,
            schema_str(
                "slack.schema.input.message.title",
                "slack.schema.input.message.description",
            ),
        )],
        true,
    )
}

fn output_schema() -> SchemaIr {
    schema_obj(
        "slack.schema.output.title",
        "slack.schema.output.description",
        vec![
            (
                "ok",
                true,
                schema_bool_ir(
                    "slack.schema.output.ok.title",
                    "slack.schema.output.ok.description",
                ),
            ),
            (
                "message_id",
                false,
                schema_str(
                    "slack.schema.output.message_id.title",
                    "slack.schema.output.message_id.description",
                ),
            ),
        ],
        true,
    )
}

fn config_schema() -> SchemaIr {
    schema_obj(
        "slack.schema.config.title",
        "slack.schema.config.description",
        vec![
            (
                "enabled",
                true,
                schema_bool_ir(
                    "slack.schema.config.enabled.title",
                    "slack.schema.config.enabled.description",
                ),
            ),
            (
                "default_channel",
                false,
                schema_str(
                    "slack.schema.config.default_channel.title",
                    "slack.schema.config.default_channel.description",
                ),
            ),
            (
                "public_base_url",
                true,
                schema_str_fmt(
                    "slack.schema.config.public_base_url.title",
                    "slack.schema.config.public_base_url.description",
                    "uri",
                ),
            ),
            (
                "api_base_url",
                true,
                schema_str_fmt(
                    "slack.schema.config.api_base_url.title",
                    "slack.schema.config.api_base_url.description",
                    "uri",
                ),
            ),
            (
                "bot_token",
                true,
                schema_secret(
                    "slack.schema.config.bot_token.title",
                    "slack.schema.config.bot_token.description",
                ),
            ),
        ],
        false,
    )
}
