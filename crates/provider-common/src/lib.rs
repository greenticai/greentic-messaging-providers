use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Common error type providers can reuse to surface failures.
#[derive(Debug, Error, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProviderError {
    #[error("validation error: {0}")]
    Validation(String),
    #[error("transport error: {0}")]
    Transport(String),
    #[error("missing secret: {name} (scope: {scope})")]
    MissingSecret {
        name: String,
        scope: String,
        remediation: String,
    },
    #[error("unknown provider error: {0}")]
    Other(String),
}

impl ProviderError {
    pub fn validation(msg: impl Into<String>) -> Self {
        ProviderError::Validation(msg.into())
    }

    pub fn transport(msg: impl Into<String>) -> Self {
        ProviderError::Transport(msg.into())
    }

    pub fn other(msg: impl Into<String>) -> Self {
        ProviderError::Other(msg.into())
    }

    pub fn missing_secret(name: impl Into<String>) -> Self {
        let name = name.into();
        ProviderError::MissingSecret {
            name: name.clone(),
            scope: "tenant".into(),
            remediation: format!(
                "Provide the `{name}` secret via greentic:secrets-store for this tenant."
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_validation_error() {
        let err = ProviderError::validation("missing token");
        assert_eq!(err, ProviderError::Validation("missing token".into()));
        assert_eq!(err.to_string(), "validation error: missing token");
    }

    #[test]
    fn builds_missing_secret_error() {
        let err = ProviderError::missing_secret("API_KEY");
        assert!(matches!(err, ProviderError::MissingSecret { .. }));
        assert_eq!(err.to_string(), "missing secret: API_KEY (scope: tenant)");
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ProviderCapabilities {
    pub supports_threads: bool,
    pub supports_buttons: bool,
    pub supports_webhook_validation: bool,
    pub supports_formatting_options: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ProviderLimits {
    pub max_text_len: u32,
    pub callback_data_max_bytes: u32,
    pub max_buttons_per_row: u32,
    pub max_button_rows: u32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ProviderMetadata {
    pub provider_id: String,
    pub display_name: String,
    pub version: String,
    pub rate_limit_hint: Option<String>,
    pub capabilities: ProviderCapabilities,
    pub limits: ProviderLimits,
}

impl ProviderMetadata {
    pub fn telegram() -> Self {
        Self {
            provider_id: "telegram".into(),
            display_name: "Telegram".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            rate_limit_hint: None,
            capabilities: ProviderCapabilities {
                supports_threads: true,
                supports_buttons: true,
                supports_webhook_validation: true,
                supports_formatting_options: true,
            },
            limits: ProviderLimits {
                max_text_len: 4000,
                callback_data_max_bytes: 64,
                max_buttons_per_row: 5,
                max_button_rows: 8,
            },
        }
    }
}
