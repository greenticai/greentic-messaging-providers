//! Setup questions and `apply-answers`.
//!
//! Three questions: `webhook_url` (optional), `webhook_secret` (secret) and
//! `api_key` (secret). The secrets leave this function only inside
//! `secrets_patch`; the returned `config` never contains them.

use crate::config::webhook_url_is_acceptable;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

pub const API_KEY_SECRET: &str = "API_CHANNEL_API_KEY";
pub const WEBHOOK_SECRET_SECRET: &str = "API_CHANNEL_WEBHOOK_SECRET";
/// Shortest API key accepted, so a guessable key cannot be configured.
pub const MIN_API_KEY_LEN: usize = 16;

/// `(question id, i18n key, required)`.
pub const SETUP_QUESTIONS: &[(&str, &str, bool)] = &[
    ("webhook_url", "api.qa.setup.webhook_url", false),
    ("webhook_secret", "api.qa.setup.webhook_secret", false),
    ("api_key", "api.qa.setup.api_key", true),
];

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ConfigOut {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webhook_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SecretsPatch {
    pub set: BTreeMap<String, String>,
    pub delete: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RemovePlan {
    pub remove_all: bool,
    pub cleanup: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApplyResult {
    pub ok: bool,
    pub config: Option<ConfigOut>,
    pub secrets_patch: Option<SecretsPatch>,
    pub remove: Option<RemovePlan>,
    pub diagnostics: Vec<String>,
    pub error: Option<String>,
}

impl ApplyResult {
    fn failure(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            config: None,
            secrets_patch: None,
            remove: None,
            diagnostics: Vec::new(),
            error: Some(error.into()),
        }
    }
}

fn answer(answers: &Value, key: &str) -> Option<String> {
    answers
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

fn existing_webhook_url(answers: &Value) -> Option<String> {
    answers
        .get("existing_config")
        .or_else(|| answers.get("config"))
        .and_then(|c| c.get("webhook_url"))
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

/// `mode` is `default`, `setup`, `upgrade` or `remove`.
pub fn apply_answers(mode: &str, answers: &Value) -> ApplyResult {
    if mode == "remove" {
        return ApplyResult {
            ok: true,
            config: None,
            secrets_patch: Some(SecretsPatch {
                set: BTreeMap::new(),
                delete: vec![API_KEY_SECRET.into(), WEBHOOK_SECRET_SECRET.into()],
            }),
            remove: Some(RemovePlan {
                remove_all: true,
                cleanup: vec![
                    "delete_config_key".into(),
                    "delete_provenance_key".into(),
                    "delete_provider_state_namespace".into(),
                    "best_effort_delete_provider_owned_secrets".into(),
                ],
            }),
            diagnostics: Vec::new(),
            error: None,
        };
    }
    let upgrade = mode == "upgrade";
    let mut secrets = BTreeMap::new();

    let answered_url = answer(answers, "webhook_url");
    let webhook_url = answered_url
        .clone()
        .or_else(|| existing_webhook_url(answers));
    if let Some(url) = &answered_url
        && !webhook_url_is_acceptable(url)
    {
        return ApplyResult::failure(
            "config validation failed: webhook_url must be an https:// URL",
        );
    }

    if let Some(key) = answer(answers, "api_key") {
        if key.len() < MIN_API_KEY_LEN {
            return ApplyResult::failure(format!(
                "config validation failed: api_key must be at least {MIN_API_KEY_LEN} characters"
            ));
        }
        secrets.insert(API_KEY_SECRET.to_string(), key);
    } else if !upgrade {
        return ApplyResult::failure("config validation failed: api_key is required");
    }

    let webhook_secret = answer(answers, "webhook_secret");
    if webhook_url.is_some() && webhook_secret.is_none() && !upgrade {
        return ApplyResult::failure(
            "config validation failed: webhook_secret is required when webhook_url is set",
        );
    }
    if let Some(secret) = webhook_secret {
        secrets.insert(WEBHOOK_SECRET_SECRET.to_string(), secret);
    }

    let enabled = answers
        .get("enabled")
        .and_then(Value::as_bool)
        .or_else(|| {
            answers
                .get("existing_config")
                .and_then(|c| c.get("enabled"))
                .and_then(Value::as_bool)
        })
        .unwrap_or(true);
    ApplyResult {
        ok: true,
        config: Some(ConfigOut {
            enabled,
            webhook_url,
        }),
        secrets_patch: (!secrets.is_empty()).then_some(SecretsPatch {
            set: secrets,
            delete: Vec::new(),
        }),
        remove: None,
        diagnostics: Vec::new(),
        error: None,
    }
}
