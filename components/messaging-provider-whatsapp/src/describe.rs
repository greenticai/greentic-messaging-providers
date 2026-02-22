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
    ("business_account_id", "whatsapp.qa.setup.business_account_id", false),
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

pub(crate) fn build_qa_spec(mode: crate::bindings::exports::greentic::component::qa::Mode) -> QaSpec {
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
    ("whatsapp.op.run.title", "Title"),
    ("whatsapp.op.run.description", "Description"),
    ("whatsapp.op.send.title", "Title"),
    ("whatsapp.op.send.description", "Description"),
    ("whatsapp.op.reply.title", "Title"),
    ("whatsapp.op.reply.description", "Description"),
    ("whatsapp.op.ingest_http.title", "Title"),
    ("whatsapp.op.ingest_http.description", "Description"),
    ("whatsapp.op.render_plan.title", "Title"),
    ("whatsapp.op.render_plan.description", "Description"),
    ("whatsapp.op.encode.title", "Title"),
    ("whatsapp.op.encode.description", "Description"),
    ("whatsapp.op.send_payload.title", "Title"),
    ("whatsapp.op.send_payload.description", "Description"),
    ("whatsapp.schema.input.title", "Title"),
    ("whatsapp.schema.input.description", "Description"),
    ("whatsapp.schema.input.message.title", "Title"),
    ("whatsapp.schema.input.message.description", "Description"),
    ("whatsapp.schema.output.title", "Title"),
    ("whatsapp.schema.output.description", "Description"),
    ("whatsapp.schema.output.ok.title", "Title"),
    ("whatsapp.schema.output.ok.description", "Description"),
    ("whatsapp.schema.output.message_id.title", "Title"),
    ("whatsapp.schema.output.message_id.description", "Description"),
    ("whatsapp.schema.config.title", "Title"),
    ("whatsapp.schema.config.description", "Description"),
    ("whatsapp.schema.config.enabled.title", "Title"),
    ("whatsapp.schema.config.enabled.description", "Description"),
    ("whatsapp.schema.config.phone_number_id.title", "Title"),
    ("whatsapp.schema.config.phone_number_id.description", "Description"),
    ("whatsapp.schema.config.public_base_url.title", "Title"),
    ("whatsapp.schema.config.public_base_url.description", "Description"),
    ("whatsapp.schema.config.business_account_id.title", "Title"),
    ("whatsapp.schema.config.business_account_id.description", "Description"),
    ("whatsapp.schema.config.api_base_url.title", "Title"),
    ("whatsapp.schema.config.api_base_url.description", "Description"),
    ("whatsapp.schema.config.api_version.title", "Title"),
    ("whatsapp.schema.config.api_version.description", "Description"),
    ("whatsapp.schema.config.token.title", "Title"),
    ("whatsapp.schema.config.token.description", "Description"),
    ("whatsapp.qa.default.title", "Title"),
    ("whatsapp.qa.setup.title", "Title"),
    ("whatsapp.qa.upgrade.title", "Title"),
    ("whatsapp.qa.remove.title", "Title"),
    ("whatsapp.qa.setup.enabled", "Enabled"),
    ("whatsapp.qa.setup.phone_number_id", "Phone Number ID"),
    ("whatsapp.qa.setup.public_base_url", "Public Base URL"),
    ("whatsapp.qa.setup.business_account_id", "Business Account ID"),
    ("whatsapp.qa.setup.api_base_url", "API Base URL"),
    ("whatsapp.qa.setup.api_version", "API Version"),
    ("whatsapp.qa.setup.token", "Token"),
];

fn input_schema() -> SchemaIr {
    schema_obj(
        "whatsapp.schema.input.title", "whatsapp.schema.input.description",
        vec![("message", true, schema_str("whatsapp.schema.input.message.title", "whatsapp.schema.input.message.description"))],
        true,
    )
}

fn output_schema() -> SchemaIr {
    schema_obj(
        "whatsapp.schema.output.title", "whatsapp.schema.output.description",
        vec![
            ("ok", true, schema_bool_ir("whatsapp.schema.output.ok.title", "whatsapp.schema.output.ok.description")),
            ("message_id", false, schema_str("whatsapp.schema.output.message_id.title", "whatsapp.schema.output.message_id.description")),
        ],
        true,
    )
}

fn config_schema() -> SchemaIr {
    schema_obj(
        "whatsapp.schema.config.title", "whatsapp.schema.config.description",
        vec![
            ("enabled", true, schema_bool_ir("whatsapp.schema.config.enabled.title", "whatsapp.schema.config.enabled.description")),
            ("phone_number_id", true, schema_str("whatsapp.schema.config.phone_number_id.title", "whatsapp.schema.config.phone_number_id.description")),
            ("public_base_url", true, schema_str_fmt("whatsapp.schema.config.public_base_url.title", "whatsapp.schema.config.public_base_url.description", "uri")),
            ("business_account_id", false, schema_str("whatsapp.schema.config.business_account_id.title", "whatsapp.schema.config.business_account_id.description")),
            ("api_base_url", true, schema_str_fmt("whatsapp.schema.config.api_base_url.title", "whatsapp.schema.config.api_base_url.description", "uri")),
            ("api_version", true, schema_str("whatsapp.schema.config.api_version.title", "whatsapp.schema.config.api_version.description")),
            ("token", false, schema_secret("whatsapp.schema.config.token.title", "whatsapp.schema.config.token.description")),
        ],
        false,
    )
}
