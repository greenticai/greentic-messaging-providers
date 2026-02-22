use crate::bindings::greentic::secrets_store::secrets_store;
use crate::{DEFAULT_API_BASE, DEFAULT_BOT_TOKEN_KEY};
use greentic_types::Destination;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderConfig {
    #[serde(default = "default_enabled")]
    pub(crate) enabled: bool,
    #[serde(default)]
    pub(crate) default_channel: Option<String>,
    pub(crate) public_base_url: String,
    #[serde(default)]
    pub(crate) api_base_url: Option<String>,
    pub(crate) bot_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ProviderConfigOut {
    pub(crate) enabled: bool,
    pub(crate) default_channel: Option<String>,
    pub(crate) public_base_url: String,
    pub(crate) api_base_url: String,
    pub(crate) bot_token: String,
}

fn default_enabled() -> bool {
    true
}

pub(crate) fn default_config_out() -> ProviderConfigOut {
    ProviderConfigOut {
        enabled: true,
        default_channel: None,
        public_base_url: String::new(),
        api_base_url: DEFAULT_API_BASE.to_string(),
        bot_token: String::new(),
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
    if config.bot_token.trim().is_empty() {
        return Err("invalid config: bot_token cannot be empty".to_string());
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

#[cfg(test)]
pub(crate) fn parse_config_bytes(bytes: &[u8]) -> Result<ProviderConfig, String> {
    serde_json::from_slice::<ProviderConfig>(bytes).map_err(|e| format!("invalid config: {e}"))
}

pub(crate) fn parse_config_value(val: &Value) -> Result<ProviderConfig, String> {
    serde_json::from_value::<ProviderConfig>(val.clone())
        .map_err(|e| format!("invalid config: {e}"))
}

pub(crate) fn load_config(input: &Value) -> Result<ProviderConfig, String> {
    if let Some(cfg) = input.get("config") {
        return parse_config_value(cfg);
    }

    let mut partial = serde_json::Map::new();
    for key in [
        "enabled",
        "default_channel",
        "public_base_url",
        "api_base_url",
        "bot_token",
    ] {
        if let Some(v) = input.get(key) {
            partial.insert(key.to_string(), v.clone());
        }
    }
    if !partial.is_empty() {
        return parse_config_value(&Value::Object(partial));
    }

    Err("missing config: expected `config` or top-level config fields".to_string())
}

pub(crate) fn resolve_bot_token(cfg: &ProviderConfig) -> String {
    if !cfg.bot_token.trim().is_empty() {
        return cfg.bot_token.clone();
    }
    get_secret_string(DEFAULT_BOT_TOKEN_KEY).unwrap_or_default()
}

pub(crate) fn get_secret_string(key: &str) -> Result<String, String> {
    match secrets_store::get(key) {
        Ok(Some(bytes)) => String::from_utf8(bytes).map_err(|_| "secret not valid utf-8".into()),
        Ok(None) => Err(format!("missing secret: {key}")),
        Err(e) => Err(format!("secret store error: {e:?}")),
    }
}

pub(crate) fn parse_destination(parsed: &Value) -> Option<Destination> {
    let to_value = parsed.get("to")?;
    if let Some(id) = to_value.as_str() {
        let trimmed = id.trim();
        if trimmed.is_empty() {
            return None;
        }
        return Some(Destination {
            id: trimmed.to_string(),
            kind: Some("channel".to_string()),
        });
    }

    let obj = to_value.as_object()?;
    let id = obj
        .get("id")
        .and_then(|value| value.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let kind = obj
        .get("kind")
        .and_then(|value| value.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    id.map(|id| Destination {
        id,
        kind: kind.or_else(|| Some("channel".to_string())),
    })
}
