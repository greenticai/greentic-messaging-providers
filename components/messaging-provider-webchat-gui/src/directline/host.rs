use base64::{Engine as _, engine::general_purpose::STANDARD};
use provider_common::redact;
use provider_common::telemetry::{self, Field, Level, event, field};
use serde_json::Value;

use crate::PROVIDER_TYPE;
use crate::bindings::greentic::http::http_client as client;
use crate::bindings::greentic::secrets_store::secrets_store;
use crate::bindings::greentic::state::state_store;
use webchat_directline_core::directline::store::{JwksFetcher, SecretStore, StateStore};

pub struct HostStateStore;

impl StateStore for HostStateStore {
    fn read(&mut self, key: &str) -> Result<Option<Vec<u8>>, String> {
        match state_store::read(key, None) {
            Ok(bytes) => {
                if bytes.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(bytes))
                }
            }
            Err(err) => {
                if err.code == "not_found" {
                    Ok(None)
                } else {
                    let detail = redact::error_message(&err.message);
                    telemetry::emit(
                        Level::Warn,
                        PROVIDER_TYPE,
                        "state read error",
                        &[
                            Field {
                                key: "code",
                                value: err.code.as_str(),
                            },
                            Field {
                                key: field::ERROR,
                                value: &detail,
                            },
                        ],
                    );
                    Err(format!("state read error: {} - {}", err.code, detail))
                }
            }
        }
    }

    fn write(&mut self, key: &str, value: &[u8]) -> Result<(), String> {
        state_store::write(key, value, None)
            .map(|_ack| ())
            .map_err(|err| {
                let detail = redact::error_message(&err.message);
                telemetry::emit(
                    Level::Warn,
                    PROVIDER_TYPE,
                    "state write error",
                    &[
                        Field {
                            key: "code",
                            value: err.code.as_str(),
                        },
                        Field {
                            key: field::ERROR,
                            value: &detail,
                        },
                    ],
                );
                format!("state write error: {} - {}", err.code, detail)
            })
    }

    fn write_if_absent(&mut self, key: &str, value: &[u8]) -> Result<Option<bool>, String> {
        match state_store::write_if_absent(key, value, None) {
            Ok(created) => Ok(Some(created)),
            Err(err) if err.code == "unsupported" => Ok(None),
            Err(err) => {
                let detail = redact::error_message(&err.message);
                telemetry::emit(
                    Level::Warn,
                    PROVIDER_TYPE,
                    "state write-if-absent error",
                    &[
                        Field {
                            key: "code",
                            value: err.code.as_str(),
                        },
                        Field {
                            key: field::ERROR,
                            value: &detail,
                        },
                    ],
                );
                Err(format!(
                    "state write-if-absent error: {} - {}",
                    err.code, detail
                ))
            }
        }
    }
}

/// Host-backed fetcher for an OIDC issuer's JWKS document over the WIT HTTP client.
pub struct HostJwksFetcher;

impl JwksFetcher for HostJwksFetcher {
    fn fetch(&self, jwks_url: &str) -> Result<String, String> {
        let request = client::Request {
            method: "GET".into(),
            url: jwks_url.to_string(),
            headers: vec![("Accept".into(), "application/json".into())],
            body: None,
        };
        let response = client::send(&request, None, None)
            .map_err(|err| format!("jwks request failed: {}", err.message))?;
        if response.status != 200 {
            return Err(format!("jwks endpoint returned {}", response.status));
        }
        String::from_utf8(response.body.unwrap_or_default())
            .map_err(|_| "jwks response not utf-8".to_string())
    }
}

/// Config-aware secret store that first checks injected config for secrets
/// before falling back to the host secrets_store interface.
///
/// The host injects secrets as base64-encoded values with `{key}_b64` suffix
/// in the request config. This allows the component to work in provider_core_only
/// mode where direct secrets_store access is denied.
pub struct ConfigAwareSecretStore {
    config: Option<Value>,
}

impl ConfigAwareSecretStore {
    pub fn new(config: Option<Value>) -> Self {
        Self { config }
    }

    /// Try to get secret from injected config first.
    /// The host injects secrets as `{key}_b64` with base64-encoded value.
    fn get_from_config(&self, key: &str) -> Option<Vec<u8>> {
        let config = self.config.as_ref()?;
        let config_obj = config.as_object()?;
        let key_b64 = format!("{}_b64", key);
        let encoded = config_obj.get(&key_b64)?.as_str()?;
        STANDARD.decode(encoded).ok()
    }
}

impl SecretStore for ConfigAwareSecretStore {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        // First check injected config
        if let Some(bytes) = self.get_from_config(key) {
            return Ok(Some(bytes));
        }

        // Fallback to host secrets_store interface
        match secrets_store::get(key) {
            Ok(opt) => Ok(opt),
            Err(err) => {
                let kind = err.name().to_string();
                let detail = redact::error_message(err.message());
                telemetry::emit(
                    Level::Warn,
                    PROVIDER_TYPE,
                    "secret fetch error",
                    &[
                        Field {
                            key: field::EVENT_KIND,
                            value: event::SECRET_FETCH,
                        },
                        Field {
                            key: field::SECRET,
                            value: key,
                        },
                        Field {
                            key: field::ERROR_KIND,
                            value: kind.as_str(),
                        },
                        Field {
                            key: field::ERROR,
                            value: &detail,
                        },
                    ],
                );
                Err(format!("secret error: {} - {}", kind, detail))
            }
        }
    }
}
