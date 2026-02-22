use crate::{PROVIDER_ID, WORLD_ID};
use provider_common::component_v0_6::{DescribePayload, RedactionRule, SchemaIr, schema_hash};
use provider_common::helpers::{
    QaQuestionDef, i18n_bundle_from_pairs, op, schema_bool_ir, schema_obj, schema_secret,
    schema_str, schema_str_fmt,
};

pub(crate) const I18N_KEYS: &[&str] = &[
    "webex.op.run.title",
    "webex.op.run.description",
    "webex.op.send.title",
    "webex.op.send.description",
    "webex.op.reply.title",
    "webex.op.reply.description",
    "webex.op.ingest_http.title",
    "webex.op.ingest_http.description",
    "webex.op.render_plan.title",
    "webex.op.render_plan.description",
    "webex.op.encode.title",
    "webex.op.encode.description",
    "webex.op.send_payload.title",
    "webex.op.send_payload.description",
    "webex.schema.input.title",
    "webex.schema.input.description",
    "webex.schema.input.message.title",
    "webex.schema.input.message.description",
    "webex.schema.output.title",
    "webex.schema.output.description",
    "webex.schema.output.ok.title",
    "webex.schema.output.ok.description",
    "webex.schema.output.message_id.title",
    "webex.schema.output.message_id.description",
    "webex.schema.config.title",
    "webex.schema.config.description",
    "webex.schema.config.enabled.title",
    "webex.schema.config.enabled.description",
    "webex.schema.config.public_base_url.title",
    "webex.schema.config.public_base_url.description",
    "webex.schema.config.default_room_id.title",
    "webex.schema.config.default_room_id.description",
    "webex.schema.config.default_to_person_email.title",
    "webex.schema.config.default_to_person_email.description",
    "webex.schema.config.api_base_url.title",
    "webex.schema.config.api_base_url.description",
    "webex.schema.config.bot_token.title",
    "webex.schema.config.bot_token.description",
    "webex.qa.default.title",
    "webex.qa.setup.title",
    "webex.qa.upgrade.title",
    "webex.qa.remove.title",
    "webex.qa.setup.enabled",
    "webex.qa.setup.public_base_url",
    "webex.qa.setup.default_room_id",
    "webex.qa.setup.default_to_person_email",
    "webex.qa.setup.api_base_url",
    "webex.qa.setup.bot_token",
];

pub(crate) const SETUP_QUESTIONS: &[QaQuestionDef] = &[
    ("enabled", "webex.qa.setup.enabled", true),
    ("public_base_url", "webex.qa.setup.public_base_url", true),
    ("default_room_id", "webex.qa.setup.default_room_id", false),
    (
        "default_to_person_email",
        "webex.qa.setup.default_to_person_email",
        false,
    ),
    ("api_base_url", "webex.qa.setup.api_base_url", true),
    ("bot_token", "webex.qa.setup.bot_token", false),
];

pub(crate) const DEFAULT_KEYS: &[&str] = &["public_base_url"];

pub(crate) fn build_describe_payload() -> DescribePayload {
    let input_schema = input_schema();
    let output_schema = output_schema();
    let config_schema = config_schema();
    DescribePayload {
        provider: PROVIDER_ID.to_string(),
        world: WORLD_ID.to_string(),
        operations: vec![
            op("run", "webex.op.run.title", "webex.op.run.description"),
            op("send", "webex.op.send.title", "webex.op.send.description"),
            op(
                "reply",
                "webex.op.reply.title",
                "webex.op.reply.description",
            ),
            op(
                "ingest_http",
                "webex.op.ingest_http.title",
                "webex.op.ingest_http.description",
            ),
            op(
                "render_plan",
                "webex.op.render_plan.title",
                "webex.op.render_plan.description",
            ),
            op(
                "encode",
                "webex.op.encode.title",
                "webex.op.encode.description",
            ),
            op(
                "send_payload",
                "webex.op.send_payload.title",
                "webex.op.send_payload.description",
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
) -> provider_common::component_v0_6::QaSpec {
    use crate::bindings::exports::greentic::component::qa::Mode;
    let mode_str = match mode {
        Mode::Default => "default",
        Mode::Setup => "setup",
        Mode::Upgrade => "upgrade",
        Mode::Remove => "remove",
    };
    provider_common::helpers::qa_spec_for_mode(mode_str, "webex", SETUP_QUESTIONS, DEFAULT_KEYS)
}

pub(crate) fn i18n_bundle(locale: String) -> Vec<u8> {
    i18n_bundle_from_pairs(
        locale,
        &[
            ("webex.op.run.title", "Run"),
            ("webex.op.run.description", "Run Webex provider operation"),
            ("webex.op.send.title", "Send"),
            ("webex.op.send.description", "Send a Webex message"),
            ("webex.op.reply.title", "Reply"),
            ("webex.op.reply.description", "Reply in a Webex thread"),
            ("webex.op.ingest_http.title", "Ingest HTTP"),
            (
                "webex.op.ingest_http.description",
                "Normalize Webex webhook payload",
            ),
            ("webex.op.render_plan.title", "Render Plan"),
            (
                "webex.op.render_plan.description",
                "Render universal message plan",
            ),
            ("webex.op.encode.title", "Encode"),
            (
                "webex.op.encode.description",
                "Encode universal payload for Webex",
            ),
            ("webex.op.send_payload.title", "Send Payload"),
            (
                "webex.op.send_payload.description",
                "Send encoded payload to Webex API",
            ),
            ("webex.schema.input.title", "Webex input"),
            (
                "webex.schema.input.description",
                "Input for Webex run/send operations",
            ),
            ("webex.schema.input.message.title", "Message"),
            ("webex.schema.input.message.description", "Message text"),
            ("webex.schema.output.title", "Webex output"),
            (
                "webex.schema.output.description",
                "Result of Webex operation",
            ),
            ("webex.schema.output.ok.title", "Success"),
            (
                "webex.schema.output.ok.description",
                "Whether operation succeeded",
            ),
            ("webex.schema.output.message_id.title", "Message ID"),
            (
                "webex.schema.output.message_id.description",
                "Webex message identifier",
            ),
            ("webex.schema.config.title", "Webex config"),
            (
                "webex.schema.config.description",
                "Webex provider configuration",
            ),
            ("webex.schema.config.enabled.title", "Enabled"),
            (
                "webex.schema.config.enabled.description",
                "Enable this provider",
            ),
            (
                "webex.schema.config.public_base_url.title",
                "Public base URL",
            ),
            (
                "webex.schema.config.public_base_url.description",
                "Public URL for callbacks",
            ),
            (
                "webex.schema.config.default_room_id.title",
                "Default room ID",
            ),
            (
                "webex.schema.config.default_room_id.description",
                "Room used when destination is omitted",
            ),
            (
                "webex.schema.config.default_to_person_email.title",
                "Default person email",
            ),
            (
                "webex.schema.config.default_to_person_email.description",
                "Email used when destination is omitted",
            ),
            ("webex.schema.config.api_base_url.title", "API base URL"),
            (
                "webex.schema.config.api_base_url.description",
                "Webex API base URL",
            ),
            ("webex.schema.config.bot_token.title", "Bot token"),
            (
                "webex.schema.config.bot_token.description",
                "Bot token for Webex API calls",
            ),
            ("webex.qa.default.title", "Default"),
            ("webex.qa.setup.title", "Setup"),
            ("webex.qa.upgrade.title", "Upgrade"),
            ("webex.qa.remove.title", "Remove"),
            ("webex.qa.setup.enabled", "Enable provider"),
            ("webex.qa.setup.public_base_url", "Public base URL"),
            ("webex.qa.setup.default_room_id", "Default room ID"),
            (
                "webex.qa.setup.default_to_person_email",
                "Default person email",
            ),
            ("webex.qa.setup.api_base_url", "API base URL"),
            ("webex.qa.setup.bot_token", "Bot token"),
        ],
    )
}

pub(crate) fn input_schema() -> SchemaIr {
    schema_obj(
        "webex.schema.input.title",
        "webex.schema.input.description",
        vec![(
            "message",
            true,
            schema_str(
                "webex.schema.input.message.title",
                "webex.schema.input.message.description",
            ),
        )],
        true,
    )
}

pub(crate) fn output_schema() -> SchemaIr {
    schema_obj(
        "webex.schema.output.title",
        "webex.schema.output.description",
        vec![
            (
                "ok",
                true,
                schema_bool_ir(
                    "webex.schema.output.ok.title",
                    "webex.schema.output.ok.description",
                ),
            ),
            (
                "message_id",
                false,
                schema_str(
                    "webex.schema.output.message_id.title",
                    "webex.schema.output.message_id.description",
                ),
            ),
        ],
        true,
    )
}

pub(crate) fn config_schema() -> SchemaIr {
    schema_obj(
        "webex.schema.config.title",
        "webex.schema.config.description",
        vec![
            (
                "enabled",
                true,
                schema_bool_ir(
                    "webex.schema.config.enabled.title",
                    "webex.schema.config.enabled.description",
                ),
            ),
            (
                "public_base_url",
                true,
                schema_str_fmt(
                    "webex.schema.config.public_base_url.title",
                    "webex.schema.config.public_base_url.description",
                    "uri",
                ),
            ),
            (
                "default_room_id",
                false,
                schema_str(
                    "webex.schema.config.default_room_id.title",
                    "webex.schema.config.default_room_id.description",
                ),
            ),
            (
                "default_to_person_email",
                false,
                schema_str(
                    "webex.schema.config.default_to_person_email.title",
                    "webex.schema.config.default_to_person_email.description",
                ),
            ),
            (
                "api_base_url",
                true,
                schema_str_fmt(
                    "webex.schema.config.api_base_url.title",
                    "webex.schema.config.api_base_url.description",
                    "uri",
                ),
            ),
            (
                "bot_token",
                false,
                schema_secret(
                    "webex.schema.config.bot_token.title",
                    "webex.schema.config.bot_token.description",
                ),
            ),
        ],
        false,
    )
}
