use provider_common::component_v0_6::{
    DescribePayload, I18nText, QaQuestionSpec, QaSpec, SchemaIr, SkipCondition, SkipExpression,
    canonical_cbor_bytes, schema_hash,
};
use provider_common::helpers::{
    op, schema_bool_ir, schema_obj, schema_secret, schema_str, schema_str_fmt,
};
use serde_json::{Value, json};

use crate::{PROVIDER_ID, WORLD_ID};

/// Helper: skip question when oauth_enabled != true
fn skip_unless_oauth() -> Option<SkipExpression> {
    Some(SkipExpression::Condition(SkipCondition {
        field: "oauth_enabled".to_string(),
        not_equals: Some(json!(true)),
        equals: None,
        is_empty: false,
        is_not_empty: false,
    }))
}

/// Helper: skip question when a specific provider toggle is not true
fn skip_unless_provider(field: &str) -> Option<SkipExpression> {
    Some(SkipExpression::Or(vec![
        // Skip if oauth not enabled
        SkipExpression::Condition(SkipCondition {
            field: "oauth_enabled".to_string(),
            not_equals: Some(json!(true)),
            equals: None,
            is_empty: false,
            is_not_empty: false,
        }),
        // Skip if this provider not enabled
        SkipExpression::Condition(SkipCondition {
            field: field.to_string(),
            not_equals: Some(json!(true)),
            equals: None,
            is_empty: false,
            is_not_empty: false,
        }),
    ]))
}

fn i18n(key: &str) -> I18nText {
    I18nText {
        key: key.to_string(),
    }
}

fn oauth_question(
    id: &str,
    label_key: &str,
    help_key: &str,
    required: bool,
    skip_if: Option<SkipExpression>,
) -> QaQuestionSpec {
    QaQuestionSpec {
        id: id.to_string(),
        label: i18n(label_key),
        help: Some(i18n(help_key)),
        help_url: None,
        error: None,
        kind: provider_common::component_v0_6::QuestionKind::Text,
        required,
        default: None,
        skip_if,
    }
}

fn oauth_secret_question(
    id: &str,
    label_key: &str,
    help_key: &str,
    skip_if: Option<SkipExpression>,
) -> QaQuestionSpec {
    QaQuestionSpec {
        id: id.to_string(),
        label: i18n(label_key),
        help: Some(i18n(help_key)),
        help_url: None,
        error: None,
        kind: provider_common::component_v0_6::QuestionKind::Text,
        required: false,
        default: None,
        skip_if,
    }
}

fn oauth_bool_question(
    id: &str,
    label_key: &str,
    help_key: &str,
    skip_if: Option<SkipExpression>,
) -> QaQuestionSpec {
    QaQuestionSpec {
        id: id.to_string(),
        label: i18n(label_key),
        help: Some(i18n(help_key)),
        help_url: None,
        error: None,
        kind: provider_common::component_v0_6::QuestionKind::Bool,
        required: false,
        default: Some(json!(false)),
        skip_if,
    }
}

fn oauth_bool_question_default_on(
    id: &str,
    label_key: &str,
    help_key: &str,
    skip_if: Option<SkipExpression>,
) -> QaQuestionSpec {
    QaQuestionSpec {
        id: id.to_string(),
        label: i18n(label_key),
        help: Some(i18n(help_key)),
        help_url: None,
        error: None,
        kind: provider_common::component_v0_6::QuestionKind::Bool,
        required: false,
        default: Some(json!(true)),
        skip_if,
    }
}

// Base setup questions (non-OAuth)
pub(crate) const BASE_SETUP_QUESTIONS: &[provider_common::helpers::QaQuestionDef] = &[
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

/// Alias for dispatch_qa_ops_with_i18n in lib.rs (uses base questions only;
/// OAuth questions are appended dynamically in build_qa_spec).
pub(crate) const SETUP_QUESTIONS: &[provider_common::helpers::QaQuestionDef] = BASE_SETUP_QUESTIONS;

/// All i18n keys — derived from I18N_PAIRS at runtime.
#[allow(dead_code)]
pub(crate) fn i18n_keys_vec() -> Vec<String> {
    I18N_PAIRS.iter().map(|(k, _)| (*k).to_string()).collect()
}

/// Static i18n keys for dispatch_qa_ops_with_i18n (must be &[&str]).
/// Includes both base keys and OAuth keys referenced by QA questions.
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
    "webchat.op.send_typing.title",
    "webchat.op.send_typing.description",
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
    "webchat.schema.config.jwt_signing_key.title",
    "webchat.schema.config.jwt_signing_key.description",
    "webchat.schema.config.oauth_greentic_issuer.title",
    "webchat.schema.config.oauth_greentic_issuer.description",
    "webchat.schema.config.oauth_greentic_client_id.title",
    "webchat.schema.config.oauth_greentic_client_id.description",
    "webchat.schema.config.oidc_issuer.title",
    "webchat.schema.config.oidc_issuer.description",
    "webchat.schema.config.oidc_audience.title",
    "webchat.schema.config.oidc_audience.description",
    "webchat.schema.config.oidc_required_scope.title",
    "webchat.schema.config.oidc_required_scope.description",
    "webchat.schema.config.oauth_google_client_id.title",
    "webchat.schema.config.oauth_google_client_id.description",
    "webchat.schema.config.oauth_google_client_secret.title",
    "webchat.schema.config.oauth_google_client_secret.description",
    "webchat.schema.config.oauth_microsoft_client_id.title",
    "webchat.schema.config.oauth_microsoft_client_id.description",
    "webchat.schema.config.oauth_microsoft_client_secret.title",
    "webchat.schema.config.oauth_microsoft_client_secret.description",
    "webchat.schema.config.oauth_github_client_id.title",
    "webchat.schema.config.oauth_github_client_id.description",
    "webchat.schema.config.oauth_github_client_secret.title",
    "webchat.schema.config.oauth_github_client_secret.description",
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
    // OAuth QA keys
    "webchat.qa.oauth.enabled",
    "webchat.qa.oauth.enabled.help",
    "webchat.qa.oauth.greentic.enable",
    "webchat.qa.oauth.greentic.enable.help",
    "webchat.qa.oauth.greentic.issuer",
    "webchat.qa.oauth.greentic.issuer.help",
    "webchat.qa.oauth.greentic.client_id",
    "webchat.qa.oauth.greentic.client_id.help",
    "webchat.qa.oauth.google.enable",
    "webchat.qa.oauth.google.enable.help",
    "webchat.qa.oauth.google.client_id",
    "webchat.qa.oauth.google.client_id.help",
    "webchat.qa.oauth.google.client_secret",
    "webchat.qa.oauth.google.client_secret.help",
    "webchat.qa.oauth.microsoft.enable",
    "webchat.qa.oauth.microsoft.enable.help",
    "webchat.qa.oauth.microsoft.client_id",
    "webchat.qa.oauth.microsoft.client_id.help",
    "webchat.qa.oauth.microsoft.client_secret",
    "webchat.qa.oauth.microsoft.client_secret.help",
    "webchat.qa.oauth.github.enable",
    "webchat.qa.oauth.github.enable.help",
    "webchat.qa.oauth.github.client_id",
    "webchat.qa.oauth.github.client_id.help",
    "webchat.qa.oauth.github.client_secret",
    "webchat.qa.oauth.github.client_secret.help",
    "webchat.qa.oauth.custom.enable",
    "webchat.qa.oauth.custom.enable.help",
    "webchat.qa.oauth.custom.label",
    "webchat.qa.oauth.custom.label.help",
    "webchat.qa.oauth.custom.auth_url",
    "webchat.qa.oauth.custom.auth_url.help",
    "webchat.qa.oauth.custom.token_url",
    "webchat.qa.oauth.custom.token_url.help",
    "webchat.qa.oauth.custom.client_id",
    "webchat.qa.oauth.custom.client_id.help",
    "webchat.qa.oauth.custom.scopes",
    "webchat.qa.oauth.custom.scopes.help",
];

/// Build OAuth-specific questions with per-provider guided flow.
fn oauth_questions() -> Vec<QaQuestionSpec> {
    vec![
        // Gate: enable OAuth?
        oauth_bool_question(
            "oauth_enabled",
            "webchat.qa.oauth.enabled",
            "webchat.qa.oauth.enabled.help",
            None,
        ),
        // ── Greentic SSO (default) ──
        oauth_bool_question_default_on(
            "oauth_enable_greentic",
            "webchat.qa.oauth.greentic.enable",
            "webchat.qa.oauth.greentic.enable.help",
            skip_unless_oauth(),
        ),
        // Not required here: validate_config_out enforces the https:// issuer
        // when Greentic SSO is on, and a gated required question breaks
        // non-interactive setup.
        oauth_question(
            "oauth_greentic_issuer",
            "webchat.qa.oauth.greentic.issuer",
            "webchat.qa.oauth.greentic.issuer.help",
            false,
            skip_unless_provider("oauth_enable_greentic"),
        ),
        oauth_question(
            "oauth_greentic_client_id",
            "webchat.qa.oauth.greentic.client_id",
            "webchat.qa.oauth.greentic.client_id.help",
            false,
            skip_unless_provider("oauth_enable_greentic"),
        ),
        // ── Google ──
        oauth_bool_question(
            "oauth_enable_google",
            "webchat.qa.oauth.google.enable",
            "webchat.qa.oauth.google.enable.help",
            skip_unless_oauth(),
        ),
        oauth_question(
            "oauth_google_client_id",
            "webchat.qa.oauth.google.client_id",
            "webchat.qa.oauth.google.client_id.help",
            false,
            skip_unless_provider("oauth_enable_google"),
        ),
        oauth_secret_question(
            "oauth_google_client_secret",
            "webchat.qa.oauth.google.client_secret",
            "webchat.qa.oauth.google.client_secret.help",
            skip_unless_provider("oauth_enable_google"),
        ),
        // ── Microsoft ──
        oauth_bool_question(
            "oauth_enable_microsoft",
            "webchat.qa.oauth.microsoft.enable",
            "webchat.qa.oauth.microsoft.enable.help",
            skip_unless_oauth(),
        ),
        oauth_question(
            "oauth_microsoft_client_id",
            "webchat.qa.oauth.microsoft.client_id",
            "webchat.qa.oauth.microsoft.client_id.help",
            false,
            skip_unless_provider("oauth_enable_microsoft"),
        ),
        oauth_secret_question(
            "oauth_microsoft_client_secret",
            "webchat.qa.oauth.microsoft.client_secret",
            "webchat.qa.oauth.microsoft.client_secret.help",
            skip_unless_provider("oauth_enable_microsoft"),
        ),
        // ── GitHub ──
        oauth_bool_question(
            "oauth_enable_github",
            "webchat.qa.oauth.github.enable",
            "webchat.qa.oauth.github.enable.help",
            skip_unless_oauth(),
        ),
        oauth_question(
            "oauth_github_client_id",
            "webchat.qa.oauth.github.client_id",
            "webchat.qa.oauth.github.client_id.help",
            false,
            skip_unless_provider("oauth_enable_github"),
        ),
        oauth_secret_question(
            "oauth_github_client_secret",
            "webchat.qa.oauth.github.client_secret",
            "webchat.qa.oauth.github.client_secret.help",
            skip_unless_provider("oauth_enable_github"),
        ),
        // ── Custom OIDC ──
        oauth_bool_question(
            "oauth_enable_custom",
            "webchat.qa.oauth.custom.enable",
            "webchat.qa.oauth.custom.enable.help",
            skip_unless_oauth(),
        ),
        oauth_question(
            "oauth_custom_label",
            "webchat.qa.oauth.custom.label",
            "webchat.qa.oauth.custom.label.help",
            false,
            skip_unless_provider("oauth_enable_custom"),
        ),
        oauth_question(
            "oauth_custom_auth_url",
            "webchat.qa.oauth.custom.auth_url",
            "webchat.qa.oauth.custom.auth_url.help",
            false,
            skip_unless_provider("oauth_enable_custom"),
        ),
        oauth_question(
            "oauth_custom_token_url",
            "webchat.qa.oauth.custom.token_url",
            "webchat.qa.oauth.custom.token_url.help",
            false,
            skip_unless_provider("oauth_enable_custom"),
        ),
        oauth_question(
            "oauth_custom_client_id",
            "webchat.qa.oauth.custom.client_id",
            "webchat.qa.oauth.custom.client_id.help",
            false,
            skip_unless_provider("oauth_enable_custom"),
        ),
        oauth_question(
            "oauth_custom_scopes",
            "webchat.qa.oauth.custom.scopes",
            "webchat.qa.oauth.custom.scopes.help",
            false,
            skip_unless_provider("oauth_enable_custom"),
        ),
    ]
}

pub(crate) const I18N_PAIRS: &[(&str, &str)] = &[
    ("webchat.op.run.title", "Run"),
    (
        "webchat.op.run.description",
        "Run WebChat provider operation",
    ),
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
    ("webchat.op.send_typing.title", "Send Typing"),
    (
        "webchat.op.send_typing.description",
        "Show that the bot is typing in the conversation; never stored as history",
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
    // OAuth gate
    ("webchat.qa.oauth.enabled", "Enable OAuth login"),
    (
        "webchat.qa.oauth.enabled.help",
        "Require users to sign in before accessing the chat",
    ),
    // Greentic SSO (default)
    ("webchat.qa.oauth.greentic.enable", "Enable Greentic SSO"),
    (
        "webchat.qa.oauth.greentic.enable.help",
        "Sign in with the Greentic identity provider. Enabled by default.",
    ),
    (
        "webchat.qa.oauth.greentic.issuer",
        "Greentic SSO issuer URL",
    ),
    (
        "webchat.qa.oauth.greentic.issuer.help",
        "Issuer base URL. Must be an https:// URL — required when Greentic SSO is enabled, since a missing issuer means every chat-token mint is rejected.",
    ),
    (
        "webchat.qa.oauth.greentic.client_id",
        "Greentic SSO client ID",
    ),
    (
        "webchat.qa.oauth.greentic.client_id.help",
        "Registered public OIDC client for this tenant. Defaults to webchat-gui.",
    ),
    // Google
    ("webchat.qa.oauth.google.enable", "Enable Google login"),
    (
        "webchat.qa.oauth.google.enable.help",
        "Allow users to sign in with their Google account",
    ),
    ("webchat.qa.oauth.google.client_id", "Google Client ID"),
    (
        "webchat.qa.oauth.google.client_id.help",
        "1. Go to https://console.cloud.google.com/apis/credentials\n2. Create OAuth 2.0 Client ID (Web application)\n3. Add your WebChat URL as Authorized redirect URI\n4. Copy the Client ID here",
    ),
    (
        "webchat.qa.oauth.google.client_secret",
        "Google Client Secret",
    ),
    (
        "webchat.qa.oauth.google.client_secret.help",
        "Copy the Client Secret from Google Cloud Console",
    ),
    // Microsoft
    (
        "webchat.qa.oauth.microsoft.enable",
        "Enable Microsoft login",
    ),
    (
        "webchat.qa.oauth.microsoft.enable.help",
        "Allow users to sign in with their Microsoft account",
    ),
    (
        "webchat.qa.oauth.microsoft.client_id",
        "Microsoft Client ID",
    ),
    (
        "webchat.qa.oauth.microsoft.client_id.help",
        "1. Go to https://portal.azure.com → App registrations\n2. Create new registration (Web, Single-page application)\n3. Add your WebChat URL as Redirect URI\n4. Copy the Application (client) ID here",
    ),
    (
        "webchat.qa.oauth.microsoft.client_secret",
        "Microsoft Client Secret",
    ),
    (
        "webchat.qa.oauth.microsoft.client_secret.help",
        "Copy from Azure App Registration → Certificates & secrets",
    ),
    // GitHub
    ("webchat.qa.oauth.github.enable", "Enable GitHub login"),
    (
        "webchat.qa.oauth.github.enable.help",
        "Allow users to sign in with their GitHub account",
    ),
    ("webchat.qa.oauth.github.client_id", "GitHub Client ID"),
    (
        "webchat.qa.oauth.github.client_id.help",
        "1. Go to https://github.com/settings/developers\n2. Create a new OAuth App\n3. Set Homepage URL and Callback URL to your WebChat URL\n4. Copy the Client ID here",
    ),
    (
        "webchat.qa.oauth.github.client_secret",
        "GitHub Client Secret",
    ),
    (
        "webchat.qa.oauth.github.client_secret.help",
        "Copy from GitHub OAuth App → Generate a new client secret",
    ),
    // Custom OIDC
    (
        "webchat.qa.oauth.custom.enable",
        "Enable custom OIDC provider",
    ),
    (
        "webchat.qa.oauth.custom.enable.help",
        "Add a custom OpenID Connect provider (Okta, Auth0, Keycloak, etc.)",
    ),
    ("webchat.qa.oauth.custom.label", "Provider display name"),
    (
        "webchat.qa.oauth.custom.label.help",
        "Name shown on the login button (e.g. 'Company SSO')",
    ),
    ("webchat.qa.oauth.custom.auth_url", "Authorization URL"),
    (
        "webchat.qa.oauth.custom.auth_url.help",
        "OIDC authorization endpoint (e.g. https://your-idp.com/authorize)",
    ),
    ("webchat.qa.oauth.custom.token_url", "Token URL"),
    (
        "webchat.qa.oauth.custom.token_url.help",
        "OIDC token endpoint (e.g. https://your-idp.com/oauth/token)",
    ),
    ("webchat.qa.oauth.custom.client_id", "Client ID"),
    (
        "webchat.qa.oauth.custom.client_id.help",
        "OAuth client ID from your identity provider",
    ),
    ("webchat.qa.oauth.custom.scopes", "Scopes"),
    (
        "webchat.qa.oauth.custom.scopes.help",
        "Space-separated scopes (default: openid profile email)",
    ),
    // Config schema
    ("webchat.schema.config.oauth_enabled.title", "OAuth enabled"),
    (
        "webchat.schema.config.oauth_enabled.description",
        "Require OAuth authentication",
    ),
    (
        "webchat.schema.config.oauth_providers.title",
        "OAuth providers",
    ),
    (
        "webchat.schema.config.oauth_providers.description",
        "JSON array of configured OAuth providers",
    ),
    (
        "webchat.schema.config.oauth_greentic_issuer.title",
        "Greentic SSO issuer",
    ),
    (
        "webchat.schema.config.oauth_greentic_issuer.description",
        "Issuer base URL for the Greentic identity provider",
    ),
    (
        "webchat.schema.config.oauth_greentic_client_id.title",
        "Greentic SSO client ID",
    ),
    (
        "webchat.schema.config.oauth_greentic_client_id.description",
        "Registered public OIDC client ID for the Greentic identity provider",
    ),
    (
        "webchat.schema.config.oidc_issuer.title",
        "OIDC issuer for chat-token verification",
    ),
    (
        "webchat.schema.config.oidc_issuer.description",
        "Pinned issuer whose access tokens may mint an identity-bound Direct Line token",
    ),
    ("webchat.schema.config.oidc_audience.title", "OIDC audience"),
    (
        "webchat.schema.config.oidc_audience.description",
        "Expected aud claim on the access token. Defaults to webchat-gui",
    ),
    (
        "webchat.schema.config.oidc_required_scope.title",
        "OIDC required scope",
    ),
    (
        "webchat.schema.config.oidc_required_scope.description",
        "Scope the access token must carry. Defaults to greentic.webchat",
    ),
    (
        "webchat.schema.config.jwt_signing_key.title",
        "JWT signing key",
    ),
    (
        "webchat.schema.config.jwt_signing_key.description",
        "Secret key used for Direct Line JWT token signing and verification",
    ),
    (
        "webchat.schema.config.oauth_google_client_id.title",
        "Google OAuth client ID",
    ),
    (
        "webchat.schema.config.oauth_google_client_id.description",
        "Client ID from Google Cloud Console for OAuth sign-in",
    ),
    (
        "webchat.schema.config.oauth_google_client_secret.title",
        "Google OAuth client secret",
    ),
    (
        "webchat.schema.config.oauth_google_client_secret.description",
        "Client secret from Google Cloud Console for OAuth sign-in",
    ),
    (
        "webchat.schema.config.oauth_microsoft_client_id.title",
        "Microsoft OAuth client ID",
    ),
    (
        "webchat.schema.config.oauth_microsoft_client_id.description",
        "Client ID from Azure App Registration for OAuth sign-in",
    ),
    (
        "webchat.schema.config.oauth_microsoft_client_secret.title",
        "Microsoft OAuth client secret",
    ),
    (
        "webchat.schema.config.oauth_microsoft_client_secret.description",
        "Client secret from Azure App Registration for OAuth sign-in",
    ),
    (
        "webchat.schema.config.oauth_github_client_id.title",
        "GitHub OAuth client ID",
    ),
    (
        "webchat.schema.config.oauth_github_client_id.description",
        "Client ID from GitHub OAuth App for sign-in",
    ),
    (
        "webchat.schema.config.oauth_github_client_secret.title",
        "GitHub OAuth client secret",
    ),
    (
        "webchat.schema.config.oauth_github_client_secret.description",
        "Client secret from GitHub OAuth App for sign-in",
    ),
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
            op(
                "send_typing",
                "webchat.op.send_typing.title",
                "webchat.op.send_typing.description",
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

    let mut spec = provider_common::helpers::qa_spec_for_mode(
        mode_str,
        "webchat",
        BASE_SETUP_QUESTIONS,
        DEFAULT_KEYS,
    );

    // Append OAuth guided questions for setup and upgrade modes
    if mode_str == "setup" || mode_str == "upgrade" {
        spec.questions.extend(oauth_questions());
    }

    spec
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
            (
                "oauth_enabled",
                false,
                schema_bool_ir(
                    "webchat.schema.config.oauth_enabled.title",
                    "webchat.schema.config.oauth_enabled.description",
                ),
            ),
            (
                "oauth_providers",
                false,
                schema_str(
                    "webchat.schema.config.oauth_providers.title",
                    "webchat.schema.config.oauth_providers.description",
                ),
            ),
            (
                "oauth_greentic_issuer",
                false,
                schema_str_fmt(
                    "webchat.schema.config.oauth_greentic_issuer.title",
                    "webchat.schema.config.oauth_greentic_issuer.description",
                    "uri",
                ),
            ),
            (
                "oauth_greentic_client_id",
                false,
                schema_str(
                    "webchat.schema.config.oauth_greentic_client_id.title",
                    "webchat.schema.config.oauth_greentic_client_id.description",
                ),
            ),
            (
                "oidc_issuer",
                false,
                schema_str(
                    "webchat.schema.config.oidc_issuer.title",
                    "webchat.schema.config.oidc_issuer.description",
                ),
            ),
            (
                "oidc_audience",
                false,
                schema_str(
                    "webchat.schema.config.oidc_audience.title",
                    "webchat.schema.config.oidc_audience.description",
                ),
            ),
            (
                "oidc_required_scope",
                false,
                schema_str(
                    "webchat.schema.config.oidc_required_scope.title",
                    "webchat.schema.config.oidc_required_scope.description",
                ),
            ),
            (
                "oauth_google_client_id",
                false,
                schema_str(
                    "webchat.schema.config.oauth_google_client_id.title",
                    "webchat.schema.config.oauth_google_client_id.description",
                ),
            ),
            (
                "oauth_google_client_secret",
                false,
                schema_secret(
                    "webchat.schema.config.oauth_google_client_secret.title",
                    "webchat.schema.config.oauth_google_client_secret.description",
                ),
            ),
            (
                "oauth_microsoft_client_id",
                false,
                schema_str(
                    "webchat.schema.config.oauth_microsoft_client_id.title",
                    "webchat.schema.config.oauth_microsoft_client_id.description",
                ),
            ),
            (
                "oauth_microsoft_client_secret",
                false,
                schema_secret(
                    "webchat.schema.config.oauth_microsoft_client_secret.title",
                    "webchat.schema.config.oauth_microsoft_client_secret.description",
                ),
            ),
            (
                "oauth_github_client_id",
                false,
                schema_str(
                    "webchat.schema.config.oauth_github_client_id.title",
                    "webchat.schema.config.oauth_github_client_id.description",
                ),
            ),
            (
                "oauth_github_client_secret",
                false,
                schema_secret(
                    "webchat.schema.config.oauth_github_client_secret.title",
                    "webchat.schema.config.oauth_github_client_secret.description",
                ),
            ),
        ],
        false,
    )
}

pub(crate) fn i18n_bundle(locale: String) -> Vec<u8> {
    let locale = if locale.trim().is_empty() {
        "en".to_string()
    } else {
        locale
    };
    let messages: serde_json::Map<String, Value> = I18N_PAIRS
        .iter()
        .map(|(key, value)| ((*key).to_string(), Value::String((*value).to_string())))
        .collect();
    canonical_cbor_bytes(&json!({"locale": locale, "messages": Value::Object(messages)}))
}
