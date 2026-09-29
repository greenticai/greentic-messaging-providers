//! Provider description and QA specs for Teams Microsoft Graph.

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
    "teams.op.ensure_channel.title",
    "teams.op.ensure_channel.description",
    "teams.op.render_plan.title",
    "teams.op.render_plan.description",
    "teams.op.encode.title",
    "teams.op.encode.description",
    "teams.op.send_payload.title",
    "teams.op.send_payload.description",
    "teams.op.send_typing.title",
    "teams.op.send_typing.description",
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
    // Config schema - Graph
    "teams.schema.config.title",
    "teams.schema.config.description",
    "teams.schema.config.enabled.title",
    "teams.schema.config.enabled.description",
    "teams.schema.config.setup_mode.title",
    "teams.schema.config.setup_mode.description",
    "teams.schema.config.public_base_url.title",
    "teams.schema.config.public_base_url.description",
    "teams.schema.config.tenant_id.title",
    "teams.schema.config.tenant_id.description",
    "teams.schema.config.client_id.title",
    "teams.schema.config.client_id.description",
    "teams.schema.config.refresh_token.title",
    "teams.schema.config.refresh_token.description",
    "teams.schema.config.access_token.title",
    "teams.schema.config.access_token.description",
    "teams.schema.config.graph_base_url.title",
    "teams.schema.config.graph_base_url.description",
    "teams.schema.config.auth_base_url.title",
    "teams.schema.config.auth_base_url.description",
    "teams.schema.config.token_scope.title",
    "teams.schema.config.token_scope.description",
    "teams.schema.config.team_id.title",
    "teams.schema.config.team_id.description",
    "teams.schema.config.team_name.title",
    "teams.schema.config.team_name.description",
    "teams.schema.config.channel_id.title",
    "teams.schema.config.channel_id.description",
    "teams.schema.config.channel_name.title",
    "teams.schema.config.channel_name.description",
    "teams.schema.config.desired_channel_name.title",
    "teams.schema.config.desired_channel_name.description",
    "teams.schema.config.messaging_endpoint.title",
    "teams.schema.config.messaging_endpoint.description",
    "teams.schema.config.skip_jwt_validation.title",
    "teams.schema.config.skip_jwt_validation.description",
    // QA titles
    "teams.qa.default.title",
    "teams.qa.setup.title",
    "teams.qa.upgrade.title",
    "teams.qa.remove.title",
    // Config labels used outside the setup question flow.
    "teams.qa.setup.enabled",
    "teams.qa.setup.setup_mode",
    "teams.qa.setup.public_base_url",
    "teams.qa.setup.tenant_id",
    "teams.qa.setup.client_id",
    "teams.qa.setup.refresh_token",
    "teams.qa.setup.access_token",
    "teams.qa.setup.graph_base_url",
    "teams.qa.setup.auth_base_url",
    "teams.qa.setup.token_scope",
    "teams.qa.setup.team_id",
    "teams.qa.setup.team_name",
    "teams.qa.setup.channel_id",
    "teams.qa.setup.channel_name",
    "teams.qa.setup.desired_channel_name",
    "teams.qa.setup.chat_id",
    "teams.qa.setup.user_id",
    "teams.qa.setup.messaging_endpoint",
];

/// QA question definitions: (id, i18n_key, required)
pub(crate) const SETUP_QUESTIONS: &[provider_common::helpers::QaQuestionDef] = &[];

/// Keys required for default/minimal setup.
pub(crate) const DEFAULT_KEYS: &[&str] = &[];

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
                "ensure_channel",
                "teams.op.ensure_channel.title",
                "teams.op.ensure_channel.description",
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
                "send_typing",
                "teams.op.send_typing.title",
                "teams.op.send_typing.description",
            ),
            // Subscription sync is provided by the Teams ingress component.
        ],
        input_schema: input_schema.clone(),
        output_schema: output_schema.clone(),
        config_schema: config_schema.clone(),
        redactions: vec![
            RedactionRule {
                path: "$.refresh_token".to_string(),
                strategy: "replace".to_string(),
            },
            RedactionRule {
                path: "$.access_token".to_string(),
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

pub(crate) const I18N_PAIRS: &[(&str, &str)] = &[
    // Operations
    ("teams.op.run.title", "Run"),
    ("teams.op.run.description", "Run Teams provider operation"),
    ("teams.op.send.title", "Send"),
    (
        "teams.op.send.description",
        "Send a Teams message via Microsoft Graph",
    ),
    ("teams.op.reply.title", "Reply"),
    (
        "teams.op.reply.description",
        "Reply in a Teams channel thread via Microsoft Graph",
    ),
    ("teams.op.ensure_channel.title", "Ensure Channel"),
    (
        "teams.op.ensure_channel.description",
        "Create the desired Teams channel when it is missing",
    ),
    ("teams.op.render_plan.title", "Render Plan"),
    (
        "teams.op.render_plan.description",
        "Render universal message plan",
    ),
    ("teams.op.encode.title", "Encode"),
    (
        "teams.op.encode.description",
        "Encode universal payload for Teams Graph API",
    ),
    ("teams.op.send_payload.title", "Send Payload"),
    (
        "teams.op.send_payload.description",
        "Send encoded payload to Microsoft Graph",
    ),
    ("teams.op.send_typing.title", "Send Typing"),
    (
        "teams.op.send_typing.description",
        "Show that the bot is typing (Bot Framework conversations only)",
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
        "Microsoft Graph message identifier",
    ),
    // Config schema - Graph
    ("teams.schema.config.title", "Teams config"),
    (
        "teams.schema.config.description",
        "Teams Graph provider configuration. This provider sends through Microsoft Graph and syncs Graph message notifications.",
    ),
    ("teams.schema.config.enabled.title", "Enabled"),
    (
        "teams.schema.config.enabled.description",
        "Enable this provider",
    ),
    ("teams.schema.config.setup_mode.title", "Setup mode"),
    (
        "teams.schema.config.setup_mode.description",
        "Teams setup mode: graph_channel for Graph channel sends/subscriptions, bot_framework for Bot Framework connector replies",
    ),
    (
        "teams.schema.config.public_base_url.title",
        "Public base URL",
    ),
    (
        "teams.schema.config.public_base_url.description",
        "Public URL for Graph notification callbacks",
    ),
    ("teams.schema.config.tenant_id.title", "Tenant ID"),
    (
        "teams.schema.config.tenant_id.description",
        "Microsoft Entra tenant ID",
    ),
    ("teams.schema.config.client_id.title", "Client ID"),
    (
        "teams.schema.config.client_id.description",
        "Microsoft Entra application client ID",
    ),
    ("teams.schema.config.refresh_token.title", "Refresh token"),
    (
        "teams.schema.config.refresh_token.description",
        "Graph OAuth refresh token",
    ),
    ("teams.schema.config.access_token.title", "Access token"),
    (
        "teams.schema.config.access_token.description",
        "Optional test/dev Graph access token override",
    ),
    ("teams.schema.config.graph_base_url.title", "Graph base URL"),
    (
        "teams.schema.config.graph_base_url.description",
        "Microsoft Graph API base URL",
    ),
    ("teams.schema.config.auth_base_url.title", "Auth base URL"),
    (
        "teams.schema.config.auth_base_url.description",
        "Microsoft identity platform authority base URL",
    ),
    ("teams.schema.config.token_scope.title", "Token scope"),
    (
        "teams.schema.config.token_scope.description",
        "OAuth token scope for Graph access tokens",
    ),
    ("teams.schema.config.team_id.title", "Team ID"),
    (
        "teams.schema.config.team_id.description",
        "Default Team identifier",
    ),
    ("teams.schema.config.team_name.title", "Team name"),
    (
        "teams.schema.config.team_name.description",
        "Human-readable display name for the selected Team. Not used for routing.",
    ),
    ("teams.schema.config.channel_id.title", "Channel ID"),
    (
        "teams.schema.config.channel_id.description",
        "Default Channel identifier",
    ),
    ("teams.schema.config.channel_name.title", "Channel name"),
    (
        "teams.schema.config.channel_name.description",
        "Human-readable display name for the selected Channel. Not used for routing.",
    ),
    (
        "teams.schema.config.desired_channel_name.title",
        "Desired channel name",
    ),
    (
        "teams.schema.config.desired_channel_name.description",
        "Desired standard channel name for setup-time create-if-missing provisioning. The resulting channel_id remains authoritative.",
    ),
    (
        "teams.schema.config.messaging_endpoint.title",
        "Messaging endpoint",
    ),
    (
        "teams.schema.config.messaging_endpoint.description",
        "Deprecated legacy field; ignored by the Graph provider",
    ),
    (
        "teams.schema.config.skip_jwt_validation.title",
        "Skip JWT validation",
    ),
    (
        "teams.schema.config.skip_jwt_validation.description",
        "Dev-only: bypass inbound JWT validation. Must not be enabled in production.",
    ),
    // QA titles
    ("teams.qa.default.title", "Default"),
    ("teams.qa.setup.title", "Setup"),
    ("teams.qa.upgrade.title", "Upgrade"),
    ("teams.qa.remove.title", "Remove"),
    // Config labels used outside the setup question flow.
    ("teams.qa.setup.enabled", "Enable provider"),
    ("teams.qa.setup.setup_mode", "Setup mode"),
    ("teams.qa.setup.public_base_url", "Public base URL"),
    ("teams.qa.setup.tenant_id", "Tenant ID"),
    ("teams.qa.setup.client_id", "Client ID"),
    ("teams.qa.setup.refresh_token", "Refresh token"),
    ("teams.qa.setup.access_token", "Access token"),
    ("teams.qa.setup.graph_base_url", "Graph base URL"),
    ("teams.qa.setup.auth_base_url", "Auth base URL"),
    ("teams.qa.setup.token_scope", "Token scope"),
    ("teams.qa.setup.team_id", "Default Team ID (optional)"),
    ("teams.qa.setup.team_name", "Default Team name (optional)"),
    ("teams.qa.setup.channel_id", "Default Channel ID (optional)"),
    (
        "teams.qa.setup.channel_name",
        "Default Channel name (optional)",
    ),
    (
        "teams.qa.setup.desired_channel_name",
        "Suggested Channel name (optional)",
    ),
    ("teams.qa.setup.chat_id", "Default Chat ID (optional)"),
    ("teams.qa.setup.user_id", "Default User ID (optional)"),
    ("teams.qa.setup.messaging_endpoint", "Messaging endpoint"),
];

pub(crate) fn i18n_bundle(locale: String) -> Vec<u8> {
    i18n_bundle_from_pairs(locale, I18N_PAIRS)
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
                "setup_mode",
                false,
                schema_str(
                    "teams.schema.config.setup_mode.title",
                    "teams.schema.config.setup_mode.description",
                ),
            ),
            (
                "public_base_url",
                false,
                schema_str_fmt(
                    "teams.schema.config.public_base_url.title",
                    "teams.schema.config.public_base_url.description",
                    "uri",
                ),
            ),
            (
                "tenant_id",
                true,
                schema_str(
                    "teams.schema.config.tenant_id.title",
                    "teams.schema.config.tenant_id.description",
                ),
            ),
            (
                "client_id",
                true,
                schema_str(
                    "teams.schema.config.client_id.title",
                    "teams.schema.config.client_id.description",
                ),
            ),
            (
                "refresh_token",
                false,
                schema_secret(
                    "teams.schema.config.refresh_token.title",
                    "teams.schema.config.refresh_token.description",
                ),
            ),
            (
                "access_token",
                false,
                schema_secret(
                    "teams.schema.config.access_token.title",
                    "teams.schema.config.access_token.description",
                ),
            ),
            (
                "graph_base_url",
                false,
                schema_str_fmt(
                    "teams.schema.config.graph_base_url.title",
                    "teams.schema.config.graph_base_url.description",
                    "uri",
                ),
            ),
            (
                "auth_base_url",
                false,
                schema_str_fmt(
                    "teams.schema.config.auth_base_url.title",
                    "teams.schema.config.auth_base_url.description",
                    "uri",
                ),
            ),
            (
                "token_scope",
                false,
                schema_str(
                    "teams.schema.config.token_scope.title",
                    "teams.schema.config.token_scope.description",
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
                "team_name",
                false,
                schema_str(
                    "teams.schema.config.team_name.title",
                    "teams.schema.config.team_name.description",
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
            (
                "channel_name",
                false,
                schema_str(
                    "teams.schema.config.channel_name.title",
                    "teams.schema.config.channel_name.description",
                ),
            ),
            (
                "desired_channel_name",
                false,
                schema_str(
                    "teams.schema.config.desired_channel_name.title",
                    "teams.schema.config.desired_channel_name.description",
                ),
            ),
            (
                "chat_id",
                false,
                schema_str("teams.qa.setup.chat_id", "teams.qa.setup.chat_id"),
            ),
            (
                "user_id",
                false,
                schema_str("teams.qa.setup.user_id", "teams.qa.setup.user_id"),
            ),
            (
                "messaging_endpoint",
                false,
                schema_str_fmt(
                    "teams.schema.config.messaging_endpoint.title",
                    "teams.schema.config.messaging_endpoint.description",
                    "uri",
                ),
            ),
            (
                "skip_jwt_validation",
                false,
                schema_bool_ir(
                    "teams.schema.config.skip_jwt_validation.title",
                    "teams.schema.config.skip_jwt_validation.description",
                ),
            ),
        ],
        false,
    )
}
