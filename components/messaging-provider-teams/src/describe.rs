//! Provider description and QA specs for Teams Bot Service.

use provider_common::component_v0_6::{
    DescribePayload, QaSpec, RedactionRule, SchemaIr, schema_hash,
};
use provider_common::helpers::{
    i18n_bundle_from_pairs, op, schema_bool_ir, schema_obj, schema_secret, schema_str,
    schema_str_fmt,
};

use crate::{PROVIDER_ID, WORLD_ID};

pub(crate) const I18N_KEYS: &[&str] = &[
    // Operations
    "teams.op.run.title",
    "teams.op.run.description",
    "teams.op.send.title",
    "teams.op.send.description",
    "teams.op.reply.title",
    "teams.op.reply.description",
    "teams.op.ingest_http.title",
    "teams.op.ingest_http.description",
    "teams.op.render_plan.title",
    "teams.op.render_plan.description",
    "teams.op.encode.title",
    "teams.op.encode.description",
    "teams.op.send_payload.title",
    "teams.op.send_payload.description",
    // Input schema
    "teams.schema.input.title",
    "teams.schema.input.description",
    "teams.schema.input.message.title",
    "teams.schema.input.message.description",
    // Output schema
    "teams.schema.output.title",
    "teams.schema.output.description",
    "teams.schema.output.ok.title",
    "teams.schema.output.ok.description",
    "teams.schema.output.message_id.title",
    "teams.schema.output.message_id.description",
    // Config schema - Bot Service
    "teams.schema.config.title",
    "teams.schema.config.description",
    "teams.schema.config.enabled.title",
    "teams.schema.config.enabled.description",
    "teams.schema.config.public_base_url.title",
    "teams.schema.config.public_base_url.description",
    "teams.schema.config.ms_bot_app_id.title",
    "teams.schema.config.ms_bot_app_id.description",
    "teams.schema.config.ms_bot_app_password.title",
    "teams.schema.config.ms_bot_app_password.description",
    "teams.schema.config.default_service_url.title",
    "teams.schema.config.default_service_url.description",
    "teams.schema.config.team_id.title",
    "teams.schema.config.team_id.description",
    "teams.schema.config.channel_id.title",
    "teams.schema.config.channel_id.description",
    // QA titles
    "teams.qa.default.title",
    "teams.qa.setup.title",
    "teams.qa.upgrade.title",
    "teams.qa.remove.title",
    // QA questions - Bot Service
    "teams.qa.setup.enabled",
    "teams.qa.setup.public_base_url",
    "teams.qa.setup.ms_bot_app_id",
    "teams.qa.setup.ms_bot_app_password",
    "teams.qa.setup.default_service_url",
    "teams.qa.setup.team_id",
    "teams.qa.setup.channel_id",
];

/// QA question definitions: (id, i18n_key, required)
pub(crate) const SETUP_QUESTIONS: &[provider_common::helpers::QaQuestionDef] = &[
    ("enabled", "teams.qa.setup.enabled", true),
    ("public_base_url", "teams.qa.setup.public_base_url", true),
    ("ms_bot_app_id", "teams.qa.setup.ms_bot_app_id", true),
    ("ms_bot_app_password", "teams.qa.setup.ms_bot_app_password", false),
    ("default_service_url", "teams.qa.setup.default_service_url", false),
    ("team_id", "teams.qa.setup.team_id", false),
    ("channel_id", "teams.qa.setup.channel_id", false),
];

/// Keys required for default/minimal setup
pub(crate) const DEFAULT_KEYS: &[&str] = &["ms_bot_app_id", "public_base_url"];

pub(crate) fn build_describe_payload() -> DescribePayload {
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
                "reply",
                "teams.op.reply.title",
                "teams.op.reply.description",
            ),
            op(
                "ingest_http",
                "teams.op.ingest_http.title",
                "teams.op.ingest_http.description",
            ),
            op(
                "render_plan",
                "teams.op.render_plan.title",
                "teams.op.render_plan.description",
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
            // Note: subscription_* operations removed - Bot Service handles subscriptions automatically
        ],
        input_schema: input_schema.clone(),
        output_schema: output_schema.clone(),
        config_schema: config_schema.clone(),
        redactions: vec![
            RedactionRule {
                path: "$.ms_bot_app_password".to_string(),
                strategy: "replace".to_string(),
            },
        ],
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
    provider_common::helpers::qa_spec_for_mode(mode_str, "teams", SETUP_QUESTIONS, DEFAULT_KEYS)
}

pub(crate) fn i18n_bundle(locale: String) -> Vec<u8> {
    i18n_bundle_from_pairs(
        locale,
        &[
            // Operations
            ("teams.op.run.title", "Run"),
            ("teams.op.run.description", "Run Teams provider operation"),
            ("teams.op.send.title", "Send"),
            ("teams.op.send.description", "Send a Teams message via Bot Connector API"),
            ("teams.op.reply.title", "Reply"),
            ("teams.op.reply.description", "Reply in a Teams thread via Bot Connector API"),
            ("teams.op.ingest_http.title", "Ingest HTTP"),
            (
                "teams.op.ingest_http.description",
                "Normalize Bot Framework Activity payload",
            ),
            ("teams.op.render_plan.title", "Render Plan"),
            (
                "teams.op.render_plan.description",
                "Render universal message plan",
            ),
            ("teams.op.encode.title", "Encode"),
            (
                "teams.op.encode.description",
                "Encode universal payload for Teams Bot Connector API",
            ),
            ("teams.op.send_payload.title", "Send Payload"),
            (
                "teams.op.send_payload.description",
                "Send encoded payload to Bot Connector API",
            ),
            // Input schema
            ("teams.schema.input.title", "Teams input"),
            (
                "teams.schema.input.description",
                "Input for Teams run/send operations",
            ),
            ("teams.schema.input.message.title", "Message"),
            ("teams.schema.input.message.description", "Message text"),
            // Output schema
            ("teams.schema.output.title", "Teams output"),
            (
                "teams.schema.output.description",
                "Result of Teams operation",
            ),
            ("teams.schema.output.ok.title", "Success"),
            (
                "teams.schema.output.ok.description",
                "Whether operation succeeded",
            ),
            ("teams.schema.output.message_id.title", "Message ID"),
            (
                "teams.schema.output.message_id.description",
                "Bot Framework activity identifier",
            ),
            // Config schema - Bot Service
            ("teams.schema.config.title", "Teams config"),
            (
                "teams.schema.config.description",
                "Teams Bot Service provider configuration",
            ),
            ("teams.schema.config.enabled.title", "Enabled"),
            (
                "teams.schema.config.enabled.description",
                "Enable this provider",
            ),
            (
                "teams.schema.config.public_base_url.title",
                "Public base URL",
            ),
            (
                "teams.schema.config.public_base_url.description",
                "Public URL for Bot Framework messaging endpoint",
            ),
            ("teams.schema.config.ms_bot_app_id.title", "Bot App ID"),
            (
                "teams.schema.config.ms_bot_app_id.description",
                "Microsoft Bot App ID from Azure Bot registration",
            ),
            ("teams.schema.config.ms_bot_app_password.title", "Bot App Password"),
            (
                "teams.schema.config.ms_bot_app_password.description",
                "Microsoft Bot App Password (client secret)",
            ),
            ("teams.schema.config.default_service_url.title", "Default Service URL"),
            (
                "teams.schema.config.default_service_url.description",
                "Default Bot Connector service URL for proactive messages",
            ),
            ("teams.schema.config.team_id.title", "Team ID"),
            (
                "teams.schema.config.team_id.description",
                "Default Team identifier",
            ),
            ("teams.schema.config.channel_id.title", "Channel ID"),
            (
                "teams.schema.config.channel_id.description",
                "Default Channel identifier",
            ),
            // QA titles
            ("teams.qa.default.title", "Default"),
            ("teams.qa.setup.title", "Setup"),
            ("teams.qa.upgrade.title", "Upgrade"),
            ("teams.qa.remove.title", "Remove"),
            // QA questions - Bot Service
            ("teams.qa.setup.enabled", "Enable provider"),
            ("teams.qa.setup.public_base_url", "Public base URL"),
            ("teams.qa.setup.ms_bot_app_id", "Microsoft Bot App ID"),
            ("teams.qa.setup.ms_bot_app_password", "Bot App Password"),
            ("teams.qa.setup.default_service_url", "Default service URL (optional)"),
            ("teams.qa.setup.team_id", "Default Team ID (optional)"),
            ("teams.qa.setup.channel_id", "Default Channel ID (optional)"),
        ],
    )
}

fn input_schema() -> SchemaIr {
    schema_obj(
        "teams.schema.input.title",
        "teams.schema.input.description",
        vec![(
            "message",
            true,
            schema_str(
                "teams.schema.input.message.title",
                "teams.schema.input.message.description",
            ),
        )],
        true,
    )
}

fn output_schema() -> SchemaIr {
    schema_obj(
        "teams.schema.output.title",
        "teams.schema.output.description",
        vec![
            (
                "ok",
                true,
                schema_bool_ir(
                    "teams.schema.output.ok.title",
                    "teams.schema.output.ok.description",
                ),
            ),
            (
                "message_id",
                false,
                schema_str(
                    "teams.schema.output.message_id.title",
                    "teams.schema.output.message_id.description",
                ),
            ),
        ],
        true,
    )
}

fn config_schema() -> SchemaIr {
    schema_obj(
        "teams.schema.config.title",
        "teams.schema.config.description",
        vec![
            (
                "enabled",
                true,
                schema_bool_ir(
                    "teams.schema.config.enabled.title",
                    "teams.schema.config.enabled.description",
                ),
            ),
            (
                "public_base_url",
                true,
                schema_str_fmt(
                    "teams.schema.config.public_base_url.title",
                    "teams.schema.config.public_base_url.description",
                    "uri",
                ),
            ),
            (
                "ms_bot_app_id",
                true,
                schema_str(
                    "teams.schema.config.ms_bot_app_id.title",
                    "teams.schema.config.ms_bot_app_id.description",
                ),
            ),
            (
                "ms_bot_app_password",
                false,
                schema_secret(
                    "teams.schema.config.ms_bot_app_password.title",
                    "teams.schema.config.ms_bot_app_password.description",
                ),
            ),
            (
                "default_service_url",
                false,
                schema_str_fmt(
                    "teams.schema.config.default_service_url.title",
                    "teams.schema.config.default_service_url.description",
                    "uri",
                ),
            ),
            (
                "team_id",
                false,
                schema_str(
                    "teams.schema.config.team_id.title",
                    "teams.schema.config.team_id.description",
                ),
            ),
            (
                "channel_id",
                false,
                schema_str(
                    "teams.schema.config.channel_id.title",
                    "teams.schema.config.channel_id.description",
                ),
            ),
        ],
        false,
    )
}
