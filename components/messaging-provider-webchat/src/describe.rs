use provider_common::component_v0_6::{DescribePayload, QaSpec, SchemaIr, schema_hash};
use provider_common::helpers::{
    op, schema_bool_ir, schema_obj, schema_str, schema_str_fmt,
};

use crate::{PROVIDER_ID, WORLD_ID};

pub(crate) const I18N_KEYS: &[&str] = &[
    "webchat.op.run.title",
    "webchat.op.run.description",
    "webchat.op.send.title",
    "webchat.op.send.description",
    "webchat.op.ingest.title",
    "webchat.op.ingest.description",
    "webchat.op.ingest_http.title",
    "webchat.op.ingest_http.description",
    "webchat.op.render_plan.title",
    "webchat.op.render_plan.description",
    "webchat.op.encode.title",
    "webchat.op.encode.description",
    "webchat.op.send_payload.title",
    "webchat.op.send_payload.description",
    "webchat.schema.input.title",
    "webchat.schema.input.description",
    "webchat.schema.input.message.title",
    "webchat.schema.input.message.description",
    "webchat.schema.output.title",
    "webchat.schema.output.description",
    "webchat.schema.output.ok.title",
    "webchat.schema.output.ok.description",
    "webchat.schema.output.message_id.title",
    "webchat.schema.output.message_id.description",
    "webchat.schema.config.title",
    "webchat.schema.config.description",
    "webchat.schema.config.enabled.title",
    "webchat.schema.config.enabled.description",
    "webchat.schema.config.public_base_url.title",
    "webchat.schema.config.public_base_url.description",
    "webchat.schema.config.mode.title",
    "webchat.schema.config.mode.description",
    "webchat.schema.config.route.title",
    "webchat.schema.config.route.description",
    "webchat.schema.config.tenant_channel_id.title",
    "webchat.schema.config.tenant_channel_id.description",
    "webchat.schema.config.base_url.title",
    "webchat.schema.config.base_url.description",
    "webchat.qa.default.title",
    "webchat.qa.setup.title",
    "webchat.qa.upgrade.title",
    "webchat.qa.remove.title",
    "webchat.qa.setup.enabled",
    "webchat.qa.setup.public_base_url",
    "webchat.qa.setup.mode",
    "webchat.qa.setup.route",
    "webchat.qa.setup.tenant_channel_id",
    "webchat.qa.setup.base_url",
];

pub(crate) const SETUP_QUESTIONS: &[provider_common::helpers::QaQuestionDef] = &[
    ("enabled", "webchat.qa.setup.enabled", true),
    ("public_base_url", "webchat.qa.setup.public_base_url", true),
    ("mode", "webchat.qa.setup.mode", true),
    ("route", "webchat.qa.setup.route", false),
    ("tenant_channel_id", "webchat.qa.setup.tenant_channel_id", false),
    ("base_url", "webchat.qa.setup.base_url", false),
];

pub(crate) const DEFAULT_KEYS: &[&str] = &["public_base_url"];

pub(crate) const I18N_PAIRS: &[(&str, &str)] = &[
    ("webchat.op.run.title", "Title"),
    ("webchat.op.run.description", "Description"),
    ("webchat.op.send.title", "Title"),
    ("webchat.op.send.description", "Description"),
    ("webchat.op.ingest.title", "Title"),
    ("webchat.op.ingest.description", "Description"),
    ("webchat.op.ingest_http.title", "Title"),
    ("webchat.op.ingest_http.description", "Description"),
    ("webchat.op.render_plan.title", "Title"),
    ("webchat.op.render_plan.description", "Description"),
    ("webchat.op.encode.title", "Title"),
    ("webchat.op.encode.description", "Description"),
    ("webchat.op.send_payload.title", "Title"),
    ("webchat.op.send_payload.description", "Description"),
    ("webchat.schema.input.title", "Title"),
    ("webchat.schema.input.description", "Description"),
    ("webchat.schema.input.message.title", "Title"),
    ("webchat.schema.input.message.description", "Description"),
    ("webchat.schema.output.title", "Title"),
    ("webchat.schema.output.description", "Description"),
    ("webchat.schema.output.ok.title", "Title"),
    ("webchat.schema.output.ok.description", "Description"),
    ("webchat.schema.output.message_id.title", "Title"),
    ("webchat.schema.output.message_id.description", "Description"),
    ("webchat.schema.config.title", "Title"),
    ("webchat.schema.config.description", "Description"),
    ("webchat.schema.config.enabled.title", "Title"),
    ("webchat.schema.config.enabled.description", "Description"),
    ("webchat.schema.config.public_base_url.title", "Title"),
    ("webchat.schema.config.public_base_url.description", "Description"),
    ("webchat.schema.config.mode.title", "Title"),
    ("webchat.schema.config.mode.description", "Description"),
    ("webchat.schema.config.route.title", "Title"),
    ("webchat.schema.config.route.description", "Description"),
    ("webchat.schema.config.tenant_channel_id.title", "Title"),
    ("webchat.schema.config.tenant_channel_id.description", "Description"),
    ("webchat.schema.config.base_url.title", "Title"),
    ("webchat.schema.config.base_url.description", "Description"),
    ("webchat.qa.default.title", "Title"),
    ("webchat.qa.setup.title", "Title"),
    ("webchat.qa.upgrade.title", "Title"),
    ("webchat.qa.remove.title", "Title"),
    ("webchat.qa.setup.enabled", "Enabled"),
    ("webchat.qa.setup.public_base_url", "Public Base URL"),
    ("webchat.qa.setup.mode", "Mode"),
    ("webchat.qa.setup.route", "Route"),
    ("webchat.qa.setup.tenant_channel_id", "Tenant Channel ID"),
    ("webchat.qa.setup.base_url", "Base URL"),
];

pub(crate) fn build_describe_payload() -> DescribePayload {
    let input_schema = input_schema();
    let output_schema = output_schema();
    let config_schema = config_schema();
    DescribePayload {
        provider: PROVIDER_ID.to_string(),
        world: WORLD_ID.to_string(),
        operations: vec![
            op("run", "webchat.op.run.title", "webchat.op.run.description"),
            op(
                "send",
                "webchat.op.send.title",
                "webchat.op.send.description",
            ),
            op(
                "ingest",
                "webchat.op.ingest.title",
                "webchat.op.ingest.description",
            ),
            op(
                "ingest_http",
                "webchat.op.ingest_http.title",
                "webchat.op.ingest_http.description",
            ),
            op(
                "render_plan",
                "webchat.op.render_plan.title",
                "webchat.op.render_plan.description",
            ),
            op(
                "encode",
                "webchat.op.encode.title",
                "webchat.op.encode.description",
            ),
            op(
                "send_payload",
                "webchat.op.send_payload.title",
                "webchat.op.send_payload.description",
            ),
        ],
        input_schema: input_schema.clone(),
        output_schema: output_schema.clone(),
        config_schema: config_schema.clone(),
        redactions: Vec::new(),
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
    provider_common::helpers::qa_spec_for_mode(mode_str, "webchat", SETUP_QUESTIONS, DEFAULT_KEYS)
}

fn input_schema() -> SchemaIr {
    schema_obj(
        "webchat.schema.input.title", "webchat.schema.input.description",
        vec![("message", true, schema_str("webchat.schema.input.message.title", "webchat.schema.input.message.description"))],
        true,
    )
}

fn output_schema() -> SchemaIr {
    schema_obj(
        "webchat.schema.output.title", "webchat.schema.output.description",
        vec![
            ("ok", true, schema_bool_ir("webchat.schema.output.ok.title", "webchat.schema.output.ok.description")),
            ("message_id", false, schema_str("webchat.schema.output.message_id.title", "webchat.schema.output.message_id.description")),
        ],
        true,
    )
}

fn config_schema() -> SchemaIr {
    schema_obj(
        "webchat.schema.config.title", "webchat.schema.config.description",
        vec![
            ("enabled", true, schema_bool_ir("webchat.schema.config.enabled.title", "webchat.schema.config.enabled.description")),
            ("public_base_url", true, schema_str_fmt("webchat.schema.config.public_base_url.title", "webchat.schema.config.public_base_url.description", "uri")),
            ("mode", true, schema_str("webchat.schema.config.mode.title", "webchat.schema.config.mode.description")),
            ("route", false, schema_str("webchat.schema.config.route.title", "webchat.schema.config.route.description")),
            ("tenant_channel_id", false, schema_str("webchat.schema.config.tenant_channel_id.title", "webchat.schema.config.tenant_channel_id.description")),
            ("base_url", false, schema_str_fmt("webchat.schema.config.base_url.title", "webchat.schema.config.base_url.description", "uri")),
        ],
        false,
    )
}
