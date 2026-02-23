use crate::auth;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderConfig {
    #[serde(default = "default_enabled")]
    pub(crate) enabled: bool,
    pub(crate) public_base_url: String,
    pub(crate) host: String,
    #[serde(default = "default_port")]
    pub(crate) port: u16,
    pub(crate) username: String,
    pub(crate) from_address: String,
    #[serde(default = "default_tls")]
    pub(crate) tls_mode: String,
    #[serde(default)]
    pub(crate) default_to_address: Option<String>,
    #[serde(default)]
    pub(crate) graph_tenant_id: Option<String>,
    #[serde(default)]
    pub(crate) graph_authority: Option<String>,
    #[serde(default)]
    pub(crate) graph_base_url: Option<String>,
    #[serde(default)]
    pub(crate) graph_token_endpoint: Option<String>,
    #[serde(default)]
    pub(crate) graph_scope: Option<String>,
    #[serde(default)]
    pub(crate) password: Option<String>,
    /// Graph API client ID (read from secrets or config).
    #[serde(default)]
    pub(crate) graph_client_id: Option<String>,
    /// Graph API client secret (optional; for client_credentials grant).
    #[serde(default)]
    pub(crate) graph_client_secret: Option<String>,
    /// Graph API refresh token (optional; for refresh_token grant).
    #[serde(default)]
    pub(crate) graph_refresh_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ProviderConfigOut {
    pub(crate) enabled: bool,
    pub(crate) public_base_url: String,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) username: String,
    pub(crate) from_address: String,
    pub(crate) tls_mode: String,
    pub(crate) default_to_address: Option<String>,
    pub(crate) password: Option<String>,
}

pub(crate) fn default_port() -> u16 {
    587
}

pub(crate) fn default_tls() -> String {
    "starttls".to_string()
}

pub(crate) fn default_enabled() -> bool {
    true
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
    for key in [
        "enabled",
        "public_base_url",
        "host",
        "port",
        "username",
        "from_address",
        "default_to_address",
        "tls_mode",
        "password",
        "graph_tenant_id",
        "graph_authority",
        "graph_base_url",
        "graph_token_endpoint",
        "graph_scope",
        "graph_client_id",
        "graph_client_secret",
        "graph_refresh_token",
    ] {
        if let Some(v) = input.get(key) {
            partial.insert(key.to_string(), v.clone());
        }
    }
    if !partial.is_empty() {
        return parse_config_value(&Value::Object(partial));
    }
    Err("config required".into())
}

pub(crate) fn default_config_out() -> ProviderConfigOut {
    ProviderConfigOut {
        enabled: true,
        public_base_url: String::new(),
        host: String::new(),
        port: default_port(),
        username: String::new(),
        from_address: String::new(),
        tls_mode: default_tls(),
        default_to_address: None,
        password: None,
    }
}

pub(crate) fn validate_config_out(config: &ProviderConfigOut) -> Result<(), String> {
    if config.public_base_url.trim().is_empty() {
        return Err("config validation failed: public_base_url is required".to_string());
    }
    if config.host.trim().is_empty() {
        return Err("config validation failed: host is required".to_string());
    }
    if config.username.trim().is_empty() {
        return Err("config validation failed: username is required".to_string());
    }
    if config.from_address.trim().is_empty() {
        return Err("config validation failed: from_address is required".to_string());
    }
    if config.port == 0 {
        return Err("config validation failed: port must be greater than zero".to_string());
    }
    if !(config.public_base_url.starts_with("http://")
        || config.public_base_url.starts_with("https://"))
    {
        return Err(
            "config validation failed: public_base_url must be an absolute URL".to_string(),
        );
    }
    Ok(())
}

pub(crate) fn validate_provider_config(cfg: ProviderConfig) -> Result<ProviderConfig, String> {
    if cfg.public_base_url.trim().is_empty() {
        return Err("invalid config: public_base_url cannot be empty".to_string());
    }
    if cfg.host.trim().is_empty() {
        return Err("invalid config: host cannot be empty".to_string());
    }
    if cfg.username.trim().is_empty() {
        return Err("invalid config: username cannot be empty".to_string());
    }
    if cfg.from_address.trim().is_empty() {
        return Err("invalid config: from_address cannot be empty".to_string());
    }
    if let Some(password) = cfg.password.as_deref() {
        let _ = password.trim();
    }
    Ok(cfg)
}

/// Build a minimal ProviderConfig from secrets for the Graph API send path.
/// This is used when the operator doesn't pass config via payload metadata.
/// Reads ALL Graph credentials in a single pass so send_payload doesn't need
/// to call the secrets store again during token acquisition.
pub(crate) fn config_from_secrets() -> Result<ProviderConfig, String> {
    let from_address = auth::get_secret_any_case("from_address")
        .or_else(|_| auth::get_secret_any_case("FROM_ADDRESS"))
        .unwrap_or_default();
    let graph_tenant_id = auth::get_secret_any_case("graph_tenant_id")
        .or_else(|_| auth::get_secret_any_case("GRAPH_TENANT_ID"))
        .or_else(|_| auth::get_secret_any_case("ms_graph_tenant_id"))
        .ok();
    let graph_client_id = auth::get_secret_any_case("ms_graph_client_id")
        .or_else(|_| auth::get_secret_any_case("graph_client_id"))
        .ok();
    let graph_client_secret = auth::get_secret_any_case("ms_graph_client_secret")
        .or_else(|_| auth::get_secret_any_case("graph_client_secret"))
        .ok();
    let graph_refresh_token = auth::get_secret_any_case("ms_graph_refresh_token")
        .or_else(|_| auth::get_secret_any_case("graph_refresh_token"))
        .ok();
    if from_address.is_empty() {
        return Err("from_address not found in secrets (seed 'from_address' secret)".to_string());
    }
    Ok(ProviderConfig {
        enabled: true,
        public_base_url: "https://localhost".to_string(),
        host: "unused".to_string(),
        port: 587,
        username: from_address.clone(),
        from_address,
        tls_mode: "starttls".to_string(),
        default_to_address: None,
        graph_tenant_id,
        graph_authority: None,
        graph_base_url: None,
        graph_token_endpoint: None,
        graph_scope: None,
        password: None,
        graph_client_id,
        graph_client_secret,
        graph_refresh_token,
    })
}
