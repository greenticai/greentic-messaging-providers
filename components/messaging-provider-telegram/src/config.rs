use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::DEFAULT_API_BASE;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderConfig {
    #[serde(default = "default_enabled")]
    pub(crate) enabled: bool,
    pub(crate) public_base_url: String,
    #[serde(default)]
    pub(crate) default_chat_id: Option<String>,
    #[serde(default)]
    pub(crate) api_base_url: Option<String>,
    #[serde(default)]
    pub(crate) bot_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ProviderConfigOut {
    pub(crate) enabled: bool,
    pub(crate) public_base_url: String,
    pub(crate) default_chat_id: Option<String>,
    pub(crate) api_base_url: String,
    pub(crate) bot_token: Option<String>,
}

fn default_enabled() -> bool {
    true
}

pub(crate) fn default_config_out() -> ProviderConfigOut {
    ProviderConfigOut {
        enabled: true,
        public_base_url: String::new(),
        default_chat_id: None,
        api_base_url: DEFAULT_API_BASE.to_string(),
        bot_token: None,
    }
}

pub(crate) fn validate_config_out(config: &ProviderConfigOut) -> Result<(), String> {
    if config.public_base_url.trim().is_empty() {
        return Err("invalid config: public_base_url cannot be empty".to_string());
    }
    if !(config.public_base_url.starts_with("http://")
        || config.public_base_url.starts_with("https://"))
    {
        return Err("invalid config: public_base_url must be an absolute URL".to_string());
    }
    if config.api_base_url.trim().is_empty() {
        return Err("invalid config: api_base_url cannot be empty".to_string());
    }
    if !(config.api_base_url.starts_with("http://") || config.api_base_url.starts_with("https://"))
    {
        return Err("invalid config: api_base_url must be an absolute URL".to_string());
    }
    Ok(())
}

pub(crate) fn parse_config_value(val: &Value) -> Result<ProviderConfig, String> {
    let cfg = serde_json::from_value::<ProviderConfig>(val.clone())
        .map_err(|e| format!("invalid config: {e}"))?;
    validate_provider_config(cfg)
}

#[cfg(test)]
pub(crate) fn parse_config_bytes(bytes: &[u8]) -> Result<ProviderConfig, String> {
    let cfg = serde_json::from_slice::<ProviderConfig>(bytes)
        .map_err(|e| format!("invalid config: {e}"))?;
    validate_provider_config(cfg)
}

pub(crate) fn load_config(input: &Value) -> Result<ProviderConfig, String> {
    if let Some(cfg) = input.get("config") {
        return parse_config_value(cfg);
    }
    let mut partial = serde_json::Map::new();
    if let Some(v) = input.get("enabled") {
        partial.insert("enabled".into(), v.clone());
    }
    if let Some(v) = input.get("public_base_url") {
        partial.insert("public_base_url".into(), v.clone());
    }
    if let Some(v) = input.get("default_chat_id") {
        partial.insert("default_chat_id".into(), v.clone());
    }
    if let Some(v) = input.get("api_base_url") {
        partial.insert("api_base_url".into(), v.clone());
    }
    if let Some(v) = input.get("bot_token") {
        partial.insert("bot_token".into(), v.clone());
    }
    if !partial.is_empty() {
        return parse_config_value(&Value::Object(partial));
    }

    Ok(ProviderConfig {
        enabled: true,
        public_base_url: "https://invalid.local".to_string(),
        default_chat_id: None,
        api_base_url: Some(DEFAULT_API_BASE.to_string()),
        bot_token: None,
    })
}

fn validate_provider_config(cfg: ProviderConfig) -> Result<ProviderConfig, String> {
    if cfg.public_base_url.trim().is_empty() {
        return Err("invalid config: public_base_url cannot be empty".to_string());
    }
    Ok(cfg)
}

pub(crate) fn get_bot_token(cfg: &ProviderConfig) -> Result<String, String> {
    use crate::TOKEN_SECRET;
    use crate::bindings::greentic::secrets_store::secrets_store;

    if let Some(token) = cfg.bot_token.clone() {
        let token = token.trim().to_string();
        if !token.is_empty() {
            return Ok(token);
        }
    }
    match secrets_store::get(TOKEN_SECRET) {
        Ok(Some(bytes)) => String::from_utf8(bytes).map_err(|_| "bot token not utf-8".to_string()),
        Ok(None) => Err(format!(
            "missing bot_token (config or secret: {TOKEN_SECRET})"
        )),
        Err(e) => Err(format!("secret store error: {e:?}")),
    }
}
