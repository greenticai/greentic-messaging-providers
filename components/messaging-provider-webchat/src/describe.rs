use provider_common::component_v0_6::{DescribePayload, QaSpec, SchemaIr, schema_hash};
use provider_common::helpers::{op, schema_bool_ir, schema_obj, schema_str, schema_str_fmt};

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
    (
        "tenant_channel_id",
        "webchat.qa.setup.tenant_channel_id",
        false,
    ),
    ("base_url", "webchat.qa.setup.base_url", false),
];

pub(crate) const DEFAULT_KEYS: &[&str] = &["public_base_url"];

pub(crate) const I18N_PAIRS: &[(&str, &str)] = &[
    ("webchat.op.run.title", "Run"),
    ("webchat.op.run.description", "Run WebChat provider operation"),
    ("webchat.op.send.title", "Send"),
    ("webchat.op.send.description", "Send a WebChat message"),
    ("webchat.op.ingest.title", "Ingest"),
    (
        "webchat.op.ingest.description",
        "Normalize WebChat activity payload",
    ),
    ("webchat.op.ingest_http.title", "Ingest HTTP"),
    (
        "webchat.op.ingest_http.description",
        "Normalize WebChat webhook payload",
    ),
    ("webchat.op.render_plan.title", "Render Plan"),
    (
        "webchat.op.render_plan.description",
        "Render universal message plan",
    ),
    ("webchat.op.encode.title", "Encode"),
    (
        "webchat.op.encode.description",
        "Encode universal payload for WebChat",
    ),
    ("webchat.op.send_payload.title", "Send Payload"),
    (
        "webchat.op.send_payload.description",
        "Send encoded payload to WebChat API",
    ),
    ("webchat.schema.input.title", "WebChat input"),
    (
        "webchat.schema.input.description",
        "Input for WebChat run/send operations",
    ),
    ("webchat.schema.input.message.title", "Message"),
    ("webchat.schema.input.message.description", "Message text"),
    ("webchat.schema.output.title", "WebChat output"),
    (
        "webchat.schema.output.description",
        "Result of WebChat operation",
    ),
    ("webchat.schema.output.ok.title", "Success"),
    (
        "webchat.schema.output.ok.description",
        "Whether operation succeeded",
    ),
    ("webchat.schema.output.message_id.title", "Message ID"),
    (
        "webchat.schema.output.message_id.description",
        "WebChat activity identifier",
    ),
    ("webchat.schema.config.title", "WebChat config"),
    (
        "webchat.schema.config.description",
        "WebChat provider configuration",
    ),
    ("webchat.schema.config.enabled.title", "Enabled"),
    (
        "webchat.schema.config.enabled.description",
        "Enable this provider",
    ),
    (
        "webchat.schema.config.public_base_url.title",
        "Public base URL",
    ),
    (
        "webchat.schema.config.public_base_url.description",
        "Public URL for callbacks",
    ),
    ("webchat.schema.config.mode.title", "Mode"),
    (
        "webchat.schema.config.mode.description",
        "WebChat connection mode",
    ),
    ("webchat.schema.config.route.title", "Route"),
    (
        "webchat.schema.config.route.description",
        "WebChat endpoint route path",
    ),
    (
        "webchat.schema.config.tenant_channel_id.title",
        "Tenant channel ID",
    ),
    (
        "webchat.schema.config.tenant_channel_id.description",
        "Channel ID for tenant isolation",
    ),
    ("webchat.schema.config.base_url.title", "Base URL"),
    (
        "webchat.schema.config.base_url.description",
        "WebChat service base URL",
    ),
    ("webchat.qa.default.title", "Default"),
    ("webchat.qa.setup.title", "Setup"),
    ("webchat.qa.upgrade.title", "Upgrade"),
    ("webchat.qa.remove.title", "Remove"),
    ("webchat.qa.setup.enabled", "Enable provider"),
    ("webchat.qa.setup.public_base_url", "Public base URL"),
    ("webchat.qa.setup.mode", "Connection mode"),
    ("webchat.qa.setup.route", "Endpoint route"),
    ("webchat.qa.setup.tenant_channel_id", "Tenant channel ID"),
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
    provider_common::helpers::qa_spec_for_mode(mode_str, "webchat", SETUP_QUESTIONS, DEFAULT_KEYS)
}

fn input_schema() -> SchemaIr {
    schema_obj(
        "webchat.schema.input.title",
        "webchat.schema.input.description",
        vec![(
            "message",
            true,
            schema_str(
                "webchat.schema.input.message.title",
                "webchat.schema.input.message.description",
            ),
        )],
        true,
    )
}

fn output_schema() -> SchemaIr {
    schema_obj(
        "webchat.schema.output.title",
        "webchat.schema.output.description",
        vec![
            (
                "ok",
                true,
                schema_bool_ir(
                    "webchat.schema.output.ok.title",
                    "webchat.schema.output.ok.description",
                ),
            ),
            (
                "message_id",
                false,
                schema_str(
                    "webchat.schema.output.message_id.title",
                    "webchat.schema.output.message_id.description",
                ),
            ),
        ],
        true,
    )
}

fn config_schema() -> SchemaIr {
    schema_obj(
        "webchat.schema.config.title",
        "webchat.schema.config.description",
        vec![
            (
                "enabled",
                true,
                schema_bool_ir(
                    "webchat.schema.config.enabled.title",
                    "webchat.schema.config.enabled.description",
                ),
            ),
            (
                "public_base_url",
                true,
                schema_str_fmt(
                    "webchat.schema.config.public_base_url.title",
                    "webchat.schema.config.public_base_url.description",
                    "uri",
                ),
            ),
            (
                "mode",
                true,
                schema_str(
                    "webchat.schema.config.mode.title",
                    "webchat.schema.config.mode.description",
                ),
            ),
            (
                "route",
                false,
                schema_str(
                    "webchat.schema.config.route.title",
                    "webchat.schema.config.route.description",
                ),
            ),
            (
                "tenant_channel_id",
                false,
                schema_str(
                    "webchat.schema.config.tenant_channel_id.title",
                    "webchat.schema.config.tenant_channel_id.description",
                ),
            ),
            (
                "base_url",
                false,
                schema_str_fmt(
                    "webchat.schema.config.base_url.title",
                    "webchat.schema.config.base_url.description",
                    "uri",
                ),
            ),
        ],
        false,
    )
}
