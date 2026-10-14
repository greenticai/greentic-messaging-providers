//! Host-backed implementations of the core seams: state store, clock,
//! randomness, outbound HTTP and secrets.

use crate::bindings::greentic::http::http_client as client;
use crate::bindings::greentic::secrets_store::secrets_store;
use crate::bindings::greentic::state::state_store;
use api_channel_core::config::ApiConfig;
use api_channel_core::env::{Env, PostOutcome};
use api_channel_core::setup::{API_KEY_SECRET, WEBHOOK_SECRET_SECRET};
use api_channel_core::store::{StateStore, StoreError};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::Value;

pub struct HostStore;

fn failed(err: &state_store::HostError) -> StoreError {
    // Host text stays server side; core never shows it to API clients.
    StoreError::Failed(format!("{}: {}", err.code, err.message))
}

impl StateStore for HostStore {
    fn read(&mut self, key: &str) -> Result<Option<Vec<u8>>, StoreError> {
        match state_store::read(key, None) {
            Ok(bytes) if bytes.is_empty() => Ok(None),
            Ok(bytes) => Ok(Some(bytes)),
            Err(err) if err.code == "not_found" => Ok(None),
            Err(err) => Err(failed(&err)),
        }
    }

    fn write(&mut self, key: &str, value: &[u8]) -> Result<(), StoreError> {
        state_store::write(key, value, None)
            .map(|_| ())
            .map_err(|err| failed(&err))
    }

    fn delete(&mut self, key: &str) -> Result<(), StoreError> {
        state_store::delete(key, None)
            .map(|_| ())
            .map_err(|err| failed(&err))
    }

    fn write_if_absent(&mut self, key: &str, value: &[u8]) -> Result<bool, StoreError> {
        match state_store::write_if_absent(key, value, None) {
            Ok(created) => Ok(created),
            Err(err) if err.code == "unsupported" => Err(StoreError::Unsupported),
            Err(err) => Err(failed(&err)),
        }
    }
}

pub struct HostEnv;

impl Env for HostEnv {
    fn now_rfc3339(&self) -> String {
        chrono::DateTime::from_timestamp(self.now_unix() as i64, 0)
            .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
            .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_string())
    }

    fn now_unix(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    fn random_id(&mut self) -> String {
        uuid::Uuid::new_v4().simple().to_string()
    }

    fn sleep_ms(&mut self, ms: u64) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }

    fn post(&mut self, url: &str, headers: &[(String, String)], body: &[u8]) -> PostOutcome {
        let request = client::Request {
            method: "POST".into(),
            url: url.to_string(),
            headers: headers.to_vec(),
            body: Some(body.to_vec()),
        };
        let options = client::RequestOptions {
            timeout_ms: Some(5_000),
            allow_insecure: Some(false),
            follow_redirects: Some(false),
        };
        match client::send(&request, Some(options), None) {
            Ok(response) if (200..300).contains(&response.status) => PostOutcome::Delivered,
            Ok(response) if response.status == 429 || response.status >= 500 => {
                PostOutcome::Retryable
            }
            Ok(_) => PostOutcome::Rejected,
            Err(_) => PostOutcome::Retryable,
        }
    }
}

/// The host injects secrets as base64 `{name}_b64` entries in the request
/// config (so the provider works where direct secret access is denied);
/// otherwise fall back to the secrets-store capability. The value goes to the
/// caller and is never logged here.
fn secret(config: Option<&Value>, name: &str) -> Option<String> {
    let injected = config
        .and_then(|c| c.get(format!("{name}_b64")))
        .and_then(Value::as_str)
        .and_then(|encoded| STANDARD.decode(encoded).ok())
        .and_then(|bytes| String::from_utf8(bytes).ok());
    let value = match injected {
        Some(value) => Some(value),
        None => match secrets_store::get(name) {
            Ok(Some(bytes)) => String::from_utf8(bytes).ok(),
            Ok(None) | Err(_) => None,
        },
    };
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

pub fn load_config(config: Option<&Value>) -> ApiConfig {
    ApiConfig {
        api_key: secret(config, API_KEY_SECRET),
        webhook_url: config
            .and_then(|c| c.get("webhook_url"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string),
        webhook_secret: secret(config, WEBHOOK_SECRET_SECRET),
    }
}
