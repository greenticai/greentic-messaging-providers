//! Configuration for Teams Bot Service provider.
//!
//! New Bot Service schema replaces the Graph API configuration.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::bindings::greentic::secrets_store::secrets_store;
use crate::{DEFAULT_BOT_APP_ID_KEY, DEFAULT_BOT_APP_PASSWORD_KEY};
use greentic_types::Destination;

/// Provider configuration for Teams Bot Service.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderConfig {
    /// Whether the provider is enabled.
    #[serde(default = "default_enabled")]
    pub(crate) enabled: bool,

    /// Public base URL for webhook callbacks.
    pub(crate) public_base_url: String,

    /// Microsoft Bot App ID (from Azure Bot registration).
    pub(crate) ms_bot_app_id: String,

    /// Microsoft Bot App Password (secret, optional in config if stored in secrets).
    #[serde(default)]
    pub(crate) ms_bot_app_password: Option<String>,

    /// Default service URL for proactive messages.
    /// Usually extracted from incoming Activity, but can be preset.
    #[serde(default)]
    pub(crate) default_service_url: Option<String>,

    /// Default Team ID for channel messages.
    #[serde(default)]
    pub(crate) team_id: Option<String>,

    /// Default Channel ID for channel messages.
    #[serde(default)]
    pub(crate) channel_id: Option<String>,
}

/// Output configuration for QA apply_answers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ProviderConfigOut {
    pub(crate) enabled: bool,
    pub(crate) public_base_url: String,
    pub(crate) ms_bot_app_id: String,
    pub(crate) ms_bot_app_password: Option<String>,
    pub(crate) default_service_url: Option<String>,
    pub(crate) team_id: Option<String>,
    pub(crate) channel_id: Option<String>,
}

fn default_enabled() -> bool {
    true
}

/// Creates a default (empty) config output.
pub(crate) fn default_config_out() -> ProviderConfigOut {
    ProviderConfigOut {
        enabled: true,
        public_base_url: String::new(),
        ms_bot_app_id: String::new(),
        ms_bot_app_password: None,
        default_service_url: None,
        team_id: None,
        channel_id: None,
    }
}

/// Validates the config output structure.
pub(crate) fn validate_config_out(config: &ProviderConfigOut) -> Result<(), String> {
    if config.ms_bot_app_id.trim().is_empty() {
        return Err("config validation failed: ms_bot_app_id is required".to_string());
    }
    if config.public_base_url.trim().is_empty() {
        return Err("config validation failed: public_base_url is required".to_string());
    }
    if !(config.public_base_url.starts_with("http://")
        || config.public_base_url.starts_with("https://"))
    {
        return Err(
            "config validation failed: public_base_url must be an absolute URL".to_string(),
        );
    }
    if let Some(ref service_url) = config.default_service_url
        && !(service_url.is_empty()
            || service_url.starts_with("http://")
            || service_url.starts_with("https://"))
    {
        return Err(
            "config validation failed: default_service_url must be an absolute URL".to_string(),
        );
    }
    Ok(())
}

/// Validates the runtime provider config.
pub(crate) fn validate_provider_config(cfg: ProviderConfig) -> Result<ProviderConfig, String> {
    if cfg.ms_bot_app_id.trim().is_empty() {
        return Err("invalid config: ms_bot_app_id cannot be empty".to_string());
    }
    if cfg.public_base_url.trim().is_empty() {
        return Err("invalid config: public_base_url cannot be empty".to_string());
    }
    Ok(cfg)
}

#[cfg(test)]
pub(crate) fn parse_config_bytes(bytes: &[u8]) -> Result<ProviderConfig, String> {
    let cfg = serde_json::from_slice::<ProviderConfig>(bytes)
        .map_err(|e| format!("invalid config: {e}"))?;
    validate_provider_config(cfg)
}

pub(crate) fn parse_config_value(val: &Value) -> Result<ProviderConfig, String> {
    let cfg = serde_json::from_value::<ProviderConfig>(val.clone())
        .map_err(|e| format!("invalid config: {e}"))?;
    validate_provider_config(cfg)
}

/// Loads config from input JSON, nested config, or secrets store.
pub(crate) fn load_config(input: &Value) -> Result<ProviderConfig, String> {
    // First try nested "config" object
    if let Some(cfg) = input.get("config") {
        return parse_config_value(cfg);
    }

    // Try to build config from top-level keys
    let mut partial = serde_json::Map::new();
    let keys = [
        "enabled",
        "public_base_url",
        "ms_bot_app_id",
        "ms_bot_app_password",
        "default_service_url",
        "team_id",
        "channel_id",
    ];
    for key in keys {
        if let Some(v) = input.get(key) {
            partial.insert(key.to_string(), v.clone());
        }
    }
    if !partial.is_empty() {
        return parse_config_value(&Value::Object(partial));
    }

    // Fall back to secret store for required fields
    load_config_from_secrets()
}

/// Retrieves a secret from the secret store.
pub(crate) fn get_secret(key: &str) -> Result<String, String> {
    match secrets_store::get(key) {
        Ok(Some(bytes)) => String::from_utf8(bytes).map_err(|_| format!("secret {key} not utf-8")),
        Ok(None) => Err(format!("missing secret: {key}")),
        Err(e) => Err(format!("secret store error: {e:?}")),
    }
}

fn get_secret_any_case(uppercase: &str) -> Result<String, String> {
    get_secret(uppercase).or_else(|_| get_secret(&uppercase.to_ascii_lowercase()))
}

/// Loads config from secrets store when no explicit config is provided.
fn load_config_from_secrets() -> Result<ProviderConfig, String> {
    let ms_bot_app_id = get_secret_any_case(DEFAULT_BOT_APP_ID_KEY).map_err(|e| {
        format!(
            "config required: ms_bot_app_id not found (tried {} and {}): {e}",
            DEFAULT_BOT_APP_ID_KEY,
            DEFAULT_BOT_APP_ID_KEY.to_ascii_lowercase()
        )
    })?;
    let ms_bot_app_password = get_secret_any_case(DEFAULT_BOT_APP_PASSWORD_KEY).ok();

    Ok(ProviderConfig {
        enabled: true,
        public_base_url: String::new(),
        ms_bot_app_id,
        ms_bot_app_password,
        default_service_url: None,
        team_id: None,
        channel_id: None,
    })
}

/// Builds a default channel destination from config.
pub(crate) fn default_channel_destination(cfg: &ProviderConfig) -> Option<Destination> {
    let team = cfg.team_id.as_ref()?;
    let channel = cfg.channel_id.as_ref()?;
    let team = team.trim();
    let channel = channel.trim();
    if team.is_empty() || channel.is_empty() {
        return None;
    }
    Some(Destination {
        id: format!("{team}:{channel}"),
        kind: Some("channel".into()),
    })
}

/// Extracts service URL from Activity or falls back to config.
pub(crate) fn get_service_url(activity: &Value, cfg: &ProviderConfig) -> Option<String> {
    // First try to get from Activity
    activity
        .get("serviceUrl")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        // Fall back to config
        .or_else(|| cfg.default_service_url.clone())
}

/// Extracts conversation ID from Activity.
pub(crate) fn get_conversation_id(activity: &Value) -> Option<String> {
    activity
        .get("conversation")
        .and_then(|c| c.get("id"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Extracts activity ID from Activity (for threading/replies).
pub(crate) fn get_activity_id(activity: &Value) -> Option<String> {
    activity
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}
