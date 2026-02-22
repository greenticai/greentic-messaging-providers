use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::bindings::greentic::secrets_store::secrets_store;
use crate::{
    DEFAULT_AUTH_BASE, DEFAULT_CLIENT_ID_KEY, DEFAULT_CLIENT_SECRET_KEY, DEFAULT_GRAPH_BASE,
    DEFAULT_REFRESH_TOKEN_KEY, DEFAULT_TENANT_ID_KEY, DEFAULT_TOKEN_SCOPE,
};
use greentic_types::Destination;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderConfig {
    #[serde(default = "default_enabled")]
    pub(crate) enabled: bool,
    pub(crate) tenant_id: String,
    pub(crate) client_id: String,
    pub(crate) public_base_url: String,
    #[serde(default)]
    pub(crate) team_id: Option<String>,
    #[serde(default)]
    pub(crate) channel_id: Option<String>,
    #[serde(default)]
    pub(crate) graph_base_url: Option<String>,
    #[serde(default)]
    pub(crate) auth_base_url: Option<String>,
    #[serde(default)]
    pub(crate) token_scope: Option<String>,
    #[serde(default)]
    pub(crate) client_secret: Option<String>,
    #[serde(default)]
    pub(crate) refresh_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ProviderConfigOut {
    pub(crate) enabled: bool,
    pub(crate) tenant_id: String,
    pub(crate) client_id: String,
    pub(crate) public_base_url: String,
    pub(crate) team_id: Option<String>,
    pub(crate) channel_id: Option<String>,
    pub(crate) graph_base_url: String,
    pub(crate) auth_base_url: String,
    pub(crate) token_scope: String,
    pub(crate) client_secret: Option<String>,
    pub(crate) refresh_token: Option<String>,
}

fn default_enabled() -> bool {
    true
}

pub(crate) fn default_config_out() -> ProviderConfigOut {
    ProviderConfigOut {
        enabled: true,
        tenant_id: String::new(),
        client_id: String::new(),
        public_base_url: String::new(),
        team_id: None,
        channel_id: None,
        graph_base_url: DEFAULT_GRAPH_BASE.to_string(),
        auth_base_url: DEFAULT_AUTH_BASE.to_string(),
        token_scope: DEFAULT_TOKEN_SCOPE.to_string(),
        client_secret: None,
        refresh_token: None,
    }
}

pub(crate) fn validate_config_out(config: &ProviderConfigOut) -> Result<(), String> {
    if config.tenant_id.trim().is_empty() {
        return Err("config validation failed: tenant_id is required".to_string());
    }
    if config.client_id.trim().is_empty() {
        return Err("config validation failed: client_id is required".to_string());
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
    if !(config.graph_base_url.starts_with("http://")
        || config.graph_base_url.starts_with("https://"))
    {
        return Err("config validation failed: graph_base_url must be an absolute URL".to_string());
    }
    if !(config.auth_base_url.starts_with("http://")
        || config.auth_base_url.starts_with("https://"))
    {
        return Err("config validation failed: auth_base_url must be an absolute URL".to_string());
    }
    Ok(())
}

pub(crate) fn validate_provider_config(cfg: ProviderConfig) -> Result<ProviderConfig, String> {
    if cfg.tenant_id.trim().is_empty() {
        return Err("invalid config: tenant_id cannot be empty".to_string());
    }
    if cfg.client_id.trim().is_empty() {
        return Err("invalid config: client_id cannot be empty".to_string());
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

pub(crate) fn load_config(input: &Value) -> Result<ProviderConfig, String> {
    if let Some(cfg) = input.get("config") {
        return parse_config_value(cfg);
    }
    let mut partial = serde_json::Map::new();
    let keys = [
        "enabled",
        "tenant_id",
        "client_id",
        "public_base_url",
        "team_id",
        "channel_id",
        "graph_base_url",
        "auth_base_url",
        "token_scope",
        "client_secret",
        "refresh_token",
    ];
    for key in keys {
        if let Some(v) = input.get(key) {
            partial.insert(key.to_string(), v.clone());
        }
    }
    if !partial.is_empty() {
        return parse_config_value(&Value::Object(partial));
    }

    // Fall back to secret store for required fields when no config is provided.
    load_config_from_secrets()
}

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

fn load_config_from_secrets() -> Result<ProviderConfig, String> {
    let tenant_id = get_secret_any_case(DEFAULT_TENANT_ID_KEY)
        .map_err(|e| format!("config required: tenant_id not found (tried {} and {}): {e}", DEFAULT_TENANT_ID_KEY, DEFAULT_TENANT_ID_KEY.to_ascii_lowercase()))?;
    let client_id = get_secret_any_case(DEFAULT_CLIENT_ID_KEY)
        .map_err(|e| format!("config required: client_id not found (tried {} and {}): {e}", DEFAULT_CLIENT_ID_KEY, DEFAULT_CLIENT_ID_KEY.to_ascii_lowercase()))?;
    Ok(ProviderConfig {
        enabled: true,
        tenant_id,
        client_id,
        public_base_url: String::new(),
        team_id: None,
        channel_id: None,
        graph_base_url: None,
        auth_base_url: None,
        token_scope: None,
        client_secret: get_secret_any_case(DEFAULT_CLIENT_SECRET_KEY).ok(),
        refresh_token: get_secret_any_case(DEFAULT_REFRESH_TOKEN_KEY).ok(),
    })
}

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
