use provider_common::component_v0_6::{DescribePayload, QaSpec, RedactionRule, SchemaIr, schema_hash};
use provider_common::helpers::{
    i18n_bundle_from_pairs, op, schema_bool_ir, schema_obj, schema_secret, schema_str,
    schema_str_fmt,
};

use crate::{PROVIDER_ID, WORLD_ID};

pub(crate) const I18N_KEYS: &[&str] = &[
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
    "teams.op.subscription_ensure.title",
    "teams.op.subscription_ensure.description",
    "teams.op.subscription_renew.title",
    "teams.op.subscription_renew.description",
    "teams.op.subscription_delete.title",
    "teams.op.subscription_delete.description",
    "teams.schema.input.title",
    "teams.schema.input.description",
    "teams.schema.input.message.title",
    "teams.schema.input.message.description",
    "teams.schema.output.title",
    "teams.schema.output.description",
    "teams.schema.output.ok.title",
    "teams.schema.output.ok.description",
    "teams.schema.output.message_id.title",
    "teams.schema.output.message_id.description",
    "teams.schema.config.title",
    "teams.schema.config.description",
    "teams.schema.config.enabled.title",
    "teams.schema.config.enabled.description",
    "teams.schema.config.tenant_id.title",
    "teams.schema.config.tenant_id.description",
    "teams.schema.config.client_id.title",
    "teams.schema.config.client_id.description",
    "teams.schema.config.public_base_url.title",
    "teams.schema.config.public_base_url.description",
    "teams.schema.config.team_id.title",
    "teams.schema.config.team_id.description",
    "teams.schema.config.channel_id.title",
    "teams.schema.config.channel_id.description",
    "teams.schema.config.graph_base_url.title",
    "teams.schema.config.graph_base_url.description",
    "teams.schema.config.auth_base_url.title",
    "teams.schema.config.auth_base_url.description",
    "teams.schema.config.token_scope.title",
    "teams.schema.config.token_scope.description",
    "teams.schema.config.client_secret.title",
    "teams.schema.config.client_secret.description",
    "teams.schema.config.refresh_token.title",
    "teams.schema.config.refresh_token.description",
    "teams.qa.default.title",
    "teams.qa.setup.title",
    "teams.qa.upgrade.title",
    "teams.qa.remove.title",
    "teams.qa.setup.enabled",
    "teams.qa.setup.tenant_id",
    "teams.qa.setup.client_id",
    "teams.qa.setup.public_base_url",
    "teams.qa.setup.graph_base_url",
    "teams.qa.setup.auth_base_url",
    "teams.qa.setup.token_scope",
    "teams.qa.setup.client_secret",
    "teams.qa.setup.refresh_token",
    "teams.qa.setup.team_id",
    "teams.qa.setup.channel_id",
];

pub(crate) const SETUP_QUESTIONS: &[provider_common::helpers::QaQuestionDef] = &[
    ("enabled", "teams.qa.setup.enabled", true),
    ("tenant_id", "teams.qa.setup.tenant_id", true),
    ("client_id", "teams.qa.setup.client_id", true),
    ("public_base_url", "teams.qa.setup.public_base_url", true),
    ("team_id", "teams.qa.setup.team_id", false),
    ("channel_id", "teams.qa.setup.channel_id", false),
    ("graph_base_url", "teams.qa.setup.graph_base_url", true),
    ("auth_base_url", "teams.qa.setup.auth_base_url", true),
    ("token_scope", "teams.qa.setup.token_scope", true),
    ("client_secret", "teams.qa.setup.client_secret", false),
    ("refresh_token", "teams.qa.setup.refresh_token", false),
];

pub(crate) const DEFAULT_KEYS: &[&str] = &["tenant_id", "client_id", "public_base_url"];

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
            op(
                "subscription_ensure",
                "teams.op.subscription_ensure.title",
                "teams.op.subscription_ensure.description",
            ),
            op(
                "subscription_renew",
                "teams.op.subscription_renew.title",
                "teams.op.subscription_renew.description",
            ),
            op(
                "subscription_delete",
                "teams.op.subscription_delete.title",
                "teams.op.subscription_delete.description",
            ),
        ],
        input_schema: input_schema.clone(),
        output_schema: output_schema.clone(),
        config_schema: config_schema.clone(),
        redactions: vec![
            RedactionRule {
                path: "$.client_secret".to_string(),
                strategy: "replace".to_string(),
            },
            RedactionRule {
                path: "$.refresh_token".to_string(),
                strategy: "replace".to_string(),
            },
        ],
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
    provider_common::helpers::qa_spec_for_mode(mode_str, "teams", SETUP_QUESTIONS, DEFAULT_KEYS)
}

pub(crate) fn i18n_bundle(locale: String) -> Vec<u8> {
    i18n_bundle_from_pairs(locale, &[
        ("teams.op.run.title", "Run"),
        ("teams.op.run.description", "Run Teams provider operation"),
        ("teams.op.send.title", "Send"),
        ("teams.op.send.description", "Send a Teams message"),
        ("teams.op.reply.title", "Reply"),
        ("teams.op.reply.description", "Reply in a Teams thread"),
        ("teams.op.ingest_http.title", "Ingest HTTP"),
        ("teams.op.ingest_http.description", "Normalize Teams webhook payload"),
        ("teams.op.render_plan.title", "Render Plan"),
        ("teams.op.render_plan.description", "Render universal message plan"),
        ("teams.op.encode.title", "Encode"),
        ("teams.op.encode.description", "Encode universal payload for Teams"),
        ("teams.op.send_payload.title", "Send Payload"),
        ("teams.op.send_payload.description", "Send encoded payload to Graph API"),
        ("teams.op.subscription_ensure.title", "Subscription Ensure"),
        ("teams.op.subscription_ensure.description", "Create or reuse a Graph subscription"),
        ("teams.op.subscription_renew.title", "Subscription Renew"),
        ("teams.op.subscription_renew.description", "Renew a Graph subscription"),
        ("teams.op.subscription_delete.title", "Subscription Delete"),
        ("teams.op.subscription_delete.description", "Delete a Graph subscription"),
        ("teams.schema.input.title", "Teams input"),
        ("teams.schema.input.description", "Input for Teams run/send operations"),
        ("teams.schema.input.message.title", "Message"),
        ("teams.schema.input.message.description", "Message text"),
        ("teams.schema.output.title", "Teams output"),
        ("teams.schema.output.description", "Result of Teams operation"),
        ("teams.schema.output.ok.title", "Success"),
        ("teams.schema.output.ok.description", "Whether operation succeeded"),
        ("teams.schema.output.message_id.title", "Message ID"),
        ("teams.schema.output.message_id.description", "Graph message identifier"),
        ("teams.schema.config.title", "Teams config"),
        ("teams.schema.config.description", "Teams provider configuration"),
        ("teams.schema.config.enabled.title", "Enabled"),
        ("teams.schema.config.enabled.description", "Enable this provider"),
        ("teams.schema.config.tenant_id.title", "Tenant ID"),
        ("teams.schema.config.tenant_id.description", "Azure AD tenant identifier"),
        ("teams.schema.config.client_id.title", "Client ID"),
        ("teams.schema.config.client_id.description", "Azure AD application client ID"),
        ("teams.schema.config.public_base_url.title", "Public base URL"),
        ("teams.schema.config.public_base_url.description", "Public URL for webhook callbacks"),
        ("teams.schema.config.team_id.title", "Team ID"),
        ("teams.schema.config.team_id.description", "Default Team identifier"),
        ("teams.schema.config.channel_id.title", "Channel ID"),
        ("teams.schema.config.channel_id.description", "Default Channel identifier"),
        ("teams.schema.config.graph_base_url.title", "Graph base URL"),
        ("teams.schema.config.graph_base_url.description", "Microsoft Graph API base URL"),
        ("teams.schema.config.auth_base_url.title", "Auth base URL"),
        ("teams.schema.config.auth_base_url.description", "Azure AD auth endpoint base URL"),
        ("teams.schema.config.token_scope.title", "Token scope"),
        ("teams.schema.config.token_scope.description", "OAuth2 token scope"),
        ("teams.schema.config.client_secret.title", "Client secret"),
        ("teams.schema.config.client_secret.description", "Azure AD client secret"),
        ("teams.schema.config.refresh_token.title", "Refresh token"),
        ("teams.schema.config.refresh_token.description", "OAuth2 refresh token"),
        ("teams.qa.default.title", "Default"),
        ("teams.qa.setup.title", "Setup"),
        ("teams.qa.upgrade.title", "Upgrade"),
        ("teams.qa.remove.title", "Remove"),
        ("teams.qa.setup.enabled", "Enable provider"),
        ("teams.qa.setup.tenant_id", "Azure AD tenant ID"),
        ("teams.qa.setup.client_id", "Azure AD client ID"),
        ("teams.qa.setup.public_base_url", "Public base URL"),
        ("teams.qa.setup.team_id", "Default Team ID"),
        ("teams.qa.setup.channel_id", "Default Channel ID"),
        ("teams.qa.setup.graph_base_url", "Graph API base URL"),
        ("teams.qa.setup.auth_base_url", "Auth endpoint base URL"),
        ("teams.qa.setup.token_scope", "Token scope"),
        ("teams.qa.setup.client_secret", "Client secret"),
        ("teams.qa.setup.refresh_token", "Refresh token"),
    ])
}

fn input_schema() -> SchemaIr {
    schema_obj(
        "teams.schema.input.title", "teams.schema.input.description",
        vec![("message", true, schema_str("teams.schema.input.message.title", "teams.schema.input.message.description"))],
        true,
    )
}

fn output_schema() -> SchemaIr {
    schema_obj(
        "teams.schema.output.title", "teams.schema.output.description",
        vec![
            ("ok", true, schema_bool_ir("teams.schema.output.ok.title", "teams.schema.output.ok.description")),
            ("message_id", false, schema_str("teams.schema.output.message_id.title", "teams.schema.output.message_id.description")),
        ],
        true,
    )
}

fn config_schema() -> SchemaIr {
    schema_obj(
        "teams.schema.config.title", "teams.schema.config.description",
        vec![
            ("enabled", true, schema_bool_ir("teams.schema.config.enabled.title", "teams.schema.config.enabled.description")),
            ("tenant_id", true, schema_str("teams.schema.config.tenant_id.title", "teams.schema.config.tenant_id.description")),
            ("client_id", true, schema_str("teams.schema.config.client_id.title", "teams.schema.config.client_id.description")),
            ("public_base_url", true, schema_str_fmt("teams.schema.config.public_base_url.title", "teams.schema.config.public_base_url.description", "uri")),
            ("team_id", false, schema_str("teams.schema.config.team_id.title", "teams.schema.config.team_id.description")),
            ("channel_id", false, schema_str("teams.schema.config.channel_id.title", "teams.schema.config.channel_id.description")),
            ("graph_base_url", true, schema_str_fmt("teams.schema.config.graph_base_url.title", "teams.schema.config.graph_base_url.description", "uri")),
            ("auth_base_url", true, schema_str_fmt("teams.schema.config.auth_base_url.title", "teams.schema.config.auth_base_url.description", "uri")),
            ("token_scope", true, schema_str("teams.schema.config.token_scope.title", "teams.schema.config.token_scope.description")),
            ("client_secret", false, schema_secret("teams.schema.config.client_secret.title", "teams.schema.config.client_secret.description")),
            ("refresh_token", false, schema_secret("teams.schema.config.refresh_token.title", "teams.schema.config.refresh_token.description")),
        ],
        false,
    )
}
