use provider_common::component_v0_6::{DescribePayload, QaSpec, RedactionRule, SchemaIr, schema_hash};
use provider_common::helpers::{
    i18n_bundle_from_pairs, op, schema_bool_ir, schema_obj, schema_secret, schema_str,
    schema_str_fmt,
};

use crate::{PROVIDER_ID, WORLD_ID};

pub(crate) const I18N_KEYS: &[&str] = &[
    "email.op.run.title",
    "email.op.run.description",
    "email.op.send.title",
    "email.op.send.description",
    "email.op.reply.title",
    "email.op.reply.description",
    "email.op.ingest_http.title",
    "email.op.ingest_http.description",
    "email.op.render_plan.title",
    "email.op.render_plan.description",
    "email.op.encode.title",
    "email.op.encode.description",
    "email.op.send_payload.title",
    "email.op.send_payload.description",
    "email.op.subscription_ensure.title",
    "email.op.subscription_ensure.description",
    "email.op.subscription_renew.title",
    "email.op.subscription_renew.description",
    "email.op.subscription_delete.title",
    "email.op.subscription_delete.description",
    "email.schema.input.title",
    "email.schema.input.description",
    "email.schema.input.message.title",
    "email.schema.input.message.description",
    "email.schema.output.title",
    "email.schema.output.description",
    "email.schema.output.ok.title",
    "email.schema.output.ok.description",
    "email.schema.output.message_id.title",
    "email.schema.output.message_id.description",
    "email.schema.config.title",
    "email.schema.config.description",
    "email.schema.config.enabled.title",
    "email.schema.config.enabled.description",
    "email.schema.config.public_base_url.title",
    "email.schema.config.public_base_url.description",
    "email.schema.config.host.title",
    "email.schema.config.host.description",
    "email.schema.config.port.title",
    "email.schema.config.port.description",
    "email.schema.config.username.title",
    "email.schema.config.username.description",
    "email.schema.config.from_address.title",
    "email.schema.config.from_address.description",
    "email.schema.config.tls_mode.title",
    "email.schema.config.tls_mode.description",
    "email.schema.config.default_to_address.title",
    "email.schema.config.default_to_address.description",
    "email.schema.config.password.title",
    "email.schema.config.password.description",
    "email.qa.default.title",
    "email.qa.setup.title",
    "email.qa.upgrade.title",
    "email.qa.remove.title",
    "email.qa.setup.enabled",
    "email.qa.setup.public_base_url",
    "email.qa.setup.host",
    "email.qa.setup.port",
    "email.qa.setup.username",
    "email.qa.setup.from_address",
    "email.qa.setup.tls_mode",
    "email.qa.setup.default_to_address",
    "email.qa.setup.password",
];

pub(crate) const SETUP_QUESTIONS: &[provider_common::helpers::QaQuestionDef] = &[
    ("enabled", "email.qa.setup.enabled", true),
    ("public_base_url", "email.qa.setup.public_base_url", true),
    ("host", "email.qa.setup.host", true),
    ("port", "email.qa.setup.port", true),
    ("username", "email.qa.setup.username", true),
    ("from_address", "email.qa.setup.from_address", true),
    ("tls_mode", "email.qa.setup.tls_mode", true),
    ("default_to_address", "email.qa.setup.default_to_address", false),
    ("password", "email.qa.setup.password", false),
];

pub(crate) const DEFAULT_KEYS: &[&str] = &["public_base_url", "host", "username", "from_address"];

pub(crate) fn build_describe_payload() -> DescribePayload {
    let input_schema = input_schema();
    let output_schema = output_schema();
    let config_schema = config_schema();
    DescribePayload {
        provider: PROVIDER_ID.to_string(),
        world: WORLD_ID.to_string(),
        operations: vec![
            op("run", "email.op.run.title", "email.op.run.description"),
            op("send", "email.op.send.title", "email.op.send.description"),
            op(
                "reply",
                "email.op.reply.title",
                "email.op.reply.description",
            ),
            op(
                "ingest_http",
                "email.op.ingest_http.title",
                "email.op.ingest_http.description",
            ),
            op(
                "render_plan",
                "email.op.render_plan.title",
                "email.op.render_plan.description",
            ),
            op(
                "encode",
                "email.op.encode.title",
                "email.op.encode.description",
            ),
            op(
                "send_payload",
                "email.op.send_payload.title",
                "email.op.send_payload.description",
            ),
            op(
                "subscription_ensure",
                "email.op.subscription_ensure.title",
                "email.op.subscription_ensure.description",
            ),
            op(
                "subscription_renew",
                "email.op.subscription_renew.title",
                "email.op.subscription_renew.description",
            ),
            op(
                "subscription_delete",
                "email.op.subscription_delete.title",
                "email.op.subscription_delete.description",
            ),
        ],
        input_schema: input_schema.clone(),
        output_schema: output_schema.clone(),
        config_schema: config_schema.clone(),
        redactions: vec![RedactionRule {
            path: "$.password".to_string(),
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
    provider_common::helpers::qa_spec_for_mode(mode_str, "email", SETUP_QUESTIONS, DEFAULT_KEYS)
}

pub(crate) fn i18n_bundle(locale: String) -> Vec<u8> {
    i18n_bundle_from_pairs(locale, &[
        ("email.op.run.title", "Run"),
        ("email.op.run.description", "Run email provider operation"),
        ("email.op.send.title", "Send"),
        ("email.op.send.description", "Send an email message"),
        ("email.op.reply.title", "Reply"),
        ("email.op.reply.description", "Reply to an email"),
        ("email.op.ingest_http.title", "Ingest HTTP"),
        ("email.op.ingest_http.description", "Normalize email webhook payload"),
        ("email.op.render_plan.title", "Render Plan"),
        ("email.op.render_plan.description", "Render universal message plan"),
        ("email.op.encode.title", "Encode"),
        ("email.op.encode.description", "Encode universal payload for email"),
        ("email.op.send_payload.title", "Send Payload"),
        ("email.op.send_payload.description", "Send encoded payload via email"),
        ("email.op.subscription_ensure.title", "Subscription Ensure"),
        ("email.op.subscription_ensure.description", "Ensure Graph subscription exists"),
        ("email.op.subscription_renew.title", "Subscription Renew"),
        ("email.op.subscription_renew.description", "Renew Graph subscription"),
        ("email.op.subscription_delete.title", "Subscription Delete"),
        ("email.op.subscription_delete.description", "Delete Graph subscription"),
        ("email.schema.input.title", "Email input"),
        ("email.schema.input.description", "Input for email run/send operations"),
        ("email.schema.input.message.title", "Message"),
        ("email.schema.input.message.description", "Message text"),
        ("email.schema.output.title", "Email output"),
        ("email.schema.output.description", "Result of email operation"),
        ("email.schema.output.ok.title", "Success"),
        ("email.schema.output.ok.description", "Whether operation succeeded"),
        ("email.schema.output.message_id.title", "Message ID"),
        ("email.schema.output.message_id.description", "Email message identifier"),
        ("email.schema.config.title", "Email config"),
        ("email.schema.config.description", "Email provider configuration"),
        ("email.schema.config.enabled.title", "Enabled"),
        ("email.schema.config.enabled.description", "Enable this provider"),
        ("email.schema.config.public_base_url.title", "Public base URL"),
        ("email.schema.config.public_base_url.description", "Public URL for callbacks"),
        ("email.schema.config.host.title", "SMTP host"),
        ("email.schema.config.host.description", "SMTP server hostname"),
        ("email.schema.config.port.title", "SMTP port"),
        ("email.schema.config.port.description", "SMTP server port number"),
        ("email.schema.config.username.title", "Username"),
        ("email.schema.config.username.description", "SMTP authentication username"),
        ("email.schema.config.from_address.title", "From address"),
        ("email.schema.config.from_address.description", "Sender email address"),
        ("email.schema.config.tls_mode.title", "TLS mode"),
        ("email.schema.config.tls_mode.description", "TLS encryption mode (starttls, tls, none)"),
        ("email.schema.config.default_to_address.title", "Default to address"),
        ("email.schema.config.default_to_address.description", "Default recipient when destination is omitted"),
        ("email.schema.config.password.title", "Password"),
        ("email.schema.config.password.description", "SMTP authentication password"),
        ("email.qa.default.title", "Default"),
        ("email.qa.setup.title", "Setup"),
        ("email.qa.upgrade.title", "Upgrade"),
        ("email.qa.remove.title", "Remove"),
        ("email.qa.setup.enabled", "Enable provider"),
        ("email.qa.setup.public_base_url", "Public base URL"),
        ("email.qa.setup.host", "SMTP host"),
        ("email.qa.setup.port", "SMTP port"),
        ("email.qa.setup.username", "Username"),
        ("email.qa.setup.from_address", "From address"),
        ("email.qa.setup.tls_mode", "TLS mode"),
        ("email.qa.setup.default_to_address", "Default to address"),
        ("email.qa.setup.password", "Password"),
    ])
}

fn input_schema() -> SchemaIr {
    schema_obj(
        "email.schema.input.title", "email.schema.input.description",
        vec![("message", true, schema_str("email.schema.input.message.title", "email.schema.input.message.description"))],
        true,
    )
}

fn output_schema() -> SchemaIr {
    schema_obj(
        "email.schema.output.title", "email.schema.output.description",
        vec![
            ("ok", true, schema_bool_ir("email.schema.output.ok.title", "email.schema.output.ok.description")),
            ("message_id", false, schema_str("email.schema.output.message_id.title", "email.schema.output.message_id.description")),
        ],
        true,
    )
}

fn config_schema() -> SchemaIr {
    schema_obj(
        "email.schema.config.title", "email.schema.config.description",
        vec![
            ("enabled", true, schema_bool_ir("email.schema.config.enabled.title", "email.schema.config.enabled.description")),
            ("public_base_url", true, schema_str_fmt("email.schema.config.public_base_url.title", "email.schema.config.public_base_url.description", "uri")),
            ("host", true, schema_str("email.schema.config.host.title", "email.schema.config.host.description")),
            ("port", true, schema_str("email.schema.config.port.title", "email.schema.config.port.description")),
            ("username", true, schema_str("email.schema.config.username.title", "email.schema.config.username.description")),
            ("from_address", true, schema_str("email.schema.config.from_address.title", "email.schema.config.from_address.description")),
            ("tls_mode", true, schema_str("email.schema.config.tls_mode.title", "email.schema.config.tls_mode.description")),
            ("default_to_address", false, schema_str("email.schema.config.default_to_address.title", "email.schema.config.default_to_address.description")),
            ("password", false, schema_secret("email.schema.config.password.title", "email.schema.config.password.description")),
        ],
        false,
    )
}
