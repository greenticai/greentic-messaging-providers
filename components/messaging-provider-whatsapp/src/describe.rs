use crate::{PROVIDER_ID, WORLD_ID};
use provider_common::component_v0_6::{
    DescribePayload, QaSpec, RedactionRule, SchemaIr, schema_hash,
};
use provider_common::helpers::{
    op, schema_bool_ir, schema_obj, schema_secret, schema_str, schema_str_fmt,
};

pub(crate) const I18N_KEYS: &[&str] = &[
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

pub(crate) const SETUP_QUESTIONS: &[provider_common::helpers::QaQuestionDef] = &[
    ("enabled", "whatsapp.qa.setup.enabled", true),
    ("phone_number_id", "whatsapp.qa.setup.phone_number_id", true),
    ("public_base_url", "whatsapp.qa.setup.public_base_url", true),
    (
        "business_account_id",
        "whatsapp.qa.setup.business_account_id",
        false,
    ),
    ("api_base_url", "whatsapp.qa.setup.api_base_url", true),
    ("api_version", "whatsapp.qa.setup.api_version", true),
    ("token", "whatsapp.qa.setup.token", false),
];

pub(crate) const DEFAULT_KEYS: &[&str] = &["phone_number_id", "public_base_url"];

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
    provider_common::helpers::qa_spec_for_mode(mode_str, "whatsapp", SETUP_QUESTIONS, DEFAULT_KEYS)
}

pub(crate) const I18N_PAIRS: &[(&str, &str)] = &[
    ("whatsapp.op.run.title", "Run"),
    (
        "whatsapp.op.run.description",
        "Run WhatsApp provider operation",
    ),
    ("whatsapp.op.send.title", "Send"),
    ("whatsapp.op.send.description", "Send a WhatsApp message"),
    ("whatsapp.op.reply.title", "Reply"),
    (
        "whatsapp.op.reply.description",
        "Reply to a WhatsApp message",
    ),
    ("whatsapp.op.ingest_http.title", "Ingest HTTP"),
    (
        "whatsapp.op.ingest_http.description",
        "Normalize WhatsApp webhook payload",
    ),
    ("whatsapp.op.render_plan.title", "Render Plan"),
    (
        "whatsapp.op.render_plan.description",
        "Render universal message plan",
    ),
    ("whatsapp.op.encode.title", "Encode"),
    (
        "whatsapp.op.encode.description",
        "Encode universal payload for WhatsApp",
    ),
    ("whatsapp.op.send_payload.title", "Send Payload"),
    (
        "whatsapp.op.send_payload.description",
        "Send encoded payload to WhatsApp API",
    ),
    ("whatsapp.schema.input.title", "WhatsApp input"),
    (
        "whatsapp.schema.input.description",
        "Input for WhatsApp run/send operations",
    ),
    ("whatsapp.schema.input.message.title", "Message"),
    ("whatsapp.schema.input.message.description", "Message text"),
    ("whatsapp.schema.output.title", "WhatsApp output"),
    (
        "whatsapp.schema.output.description",
        "Result of WhatsApp operation",
    ),
    ("whatsapp.schema.output.ok.title", "Success"),
    (
        "whatsapp.schema.output.ok.description",
        "Whether operation succeeded",
    ),
    ("whatsapp.schema.output.message_id.title", "Message ID"),
    (
        "whatsapp.schema.output.message_id.description",
        "WhatsApp message identifier",
    ),
    ("whatsapp.schema.config.title", "WhatsApp config"),
    (
        "whatsapp.schema.config.description",
        "WhatsApp provider configuration",
    ),
    ("whatsapp.schema.config.enabled.title", "Enabled"),
    (
        "whatsapp.schema.config.enabled.description",
        "Enable this provider",
    ),
    (
        "whatsapp.schema.config.phone_number_id.title",
        "Phone number ID",
    ),
    (
        "whatsapp.schema.config.phone_number_id.description",
        "WhatsApp Business phone number ID",
    ),
    (
        "whatsapp.schema.config.public_base_url.title",
        "Public base URL",
    ),
    (
        "whatsapp.schema.config.public_base_url.description",
        "Public URL for webhook callbacks",
    ),
    (
        "whatsapp.schema.config.business_account_id.title",
        "Business account ID",
    ),
    (
        "whatsapp.schema.config.business_account_id.description",
        "WhatsApp Business account identifier",
    ),
    ("whatsapp.schema.config.api_base_url.title", "API base URL"),
    (
        "whatsapp.schema.config.api_base_url.description",
        "WhatsApp Cloud API base URL",
    ),
    ("whatsapp.schema.config.api_version.title", "API version"),
    (
        "whatsapp.schema.config.api_version.description",
        "WhatsApp Cloud API version",
    ),
    ("whatsapp.schema.config.token.title", "Access token"),
    (
        "whatsapp.schema.config.token.description",
        "Access token for WhatsApp API calls",
    ),
    ("whatsapp.qa.default.title", "Default"),
    ("whatsapp.qa.setup.title", "Setup"),
    ("whatsapp.qa.upgrade.title", "Upgrade"),
    ("whatsapp.qa.remove.title", "Remove"),
    ("whatsapp.qa.setup.enabled", "Enable provider"),
    ("whatsapp.qa.setup.phone_number_id", "Phone number ID"),
    ("whatsapp.qa.setup.public_base_url", "Public base URL"),
    (
        "whatsapp.qa.setup.business_account_id",
        "Business account ID",
    ),
    ("whatsapp.qa.setup.api_base_url", "API base URL"),
    ("whatsapp.qa.setup.api_version", "API version"),
    ("whatsapp.qa.setup.token", "Access token"),
];

fn input_schema() -> SchemaIr {
    schema_obj(
        "whatsapp.schema.input.title",
        "whatsapp.schema.input.description",
        vec![(
            "message",
            true,
            schema_str(
                "whatsapp.schema.input.message.title",
                "whatsapp.schema.input.message.description",
            ),
        )],
        true,
    )
}

fn output_schema() -> SchemaIr {
    schema_obj(
        "whatsapp.schema.output.title",
        "whatsapp.schema.output.description",
        vec![
            (
                "ok",
                true,
                schema_bool_ir(
                    "whatsapp.schema.output.ok.title",
                    "whatsapp.schema.output.ok.description",
                ),
            ),
            (
                "message_id",
                false,
                schema_str(
                    "whatsapp.schema.output.message_id.title",
                    "whatsapp.schema.output.message_id.description",
                ),
            ),
        ],
        true,
    )
}

fn config_schema() -> SchemaIr {
    schema_obj(
        "whatsapp.schema.config.title",
        "whatsapp.schema.config.description",
        vec![
            (
                "enabled",
                true,
                schema_bool_ir(
                    "whatsapp.schema.config.enabled.title",
                    "whatsapp.schema.config.enabled.description",
                ),
            ),
            (
                "phone_number_id",
                true,
                schema_str(
                    "whatsapp.schema.config.phone_number_id.title",
                    "whatsapp.schema.config.phone_number_id.description",
                ),
            ),
            (
                "public_base_url",
                true,
                schema_str_fmt(
                    "whatsapp.schema.config.public_base_url.title",
                    "whatsapp.schema.config.public_base_url.description",
                    "uri",
                ),
            ),
            (
                "business_account_id",
                false,
                schema_str(
                    "whatsapp.schema.config.business_account_id.title",
                    "whatsapp.schema.config.business_account_id.description",
                ),
            ),
            (
                "api_base_url",
                true,
                schema_str_fmt(
                    "whatsapp.schema.config.api_base_url.title",
                    "whatsapp.schema.config.api_base_url.description",
                    "uri",
                ),
            ),
            (
                "api_version",
                true,
                schema_str(
                    "whatsapp.schema.config.api_version.title",
                    "whatsapp.schema.config.api_version.description",
                ),
            ),
            (
                "token",
                false,
                schema_secret(
                    "whatsapp.schema.config.token.title",
                    "whatsapp.schema.config.token.description",
                ),
            ),
        ],
        false,
    )
}
