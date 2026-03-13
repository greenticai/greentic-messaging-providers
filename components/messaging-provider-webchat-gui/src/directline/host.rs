use crate::bindings::greentic::secrets_store::secrets_store;
use crate::bindings::greentic::state::state_store;
use webchat_directline_core::directline::store::{SecretStore, StateStore};

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
                    Err(format!("state read error: {} - {}", err.code, err.message))
                }
            }
        }
    }

    fn write(&mut self, key: &str, value: &[u8]) -> Result<(), String> {
        state_store::write(key, value, None)
            .map(|_ack| ())
            .map_err(|err| format!("state write error: {} - {}", err.code, err.message))
    }
}

pub struct HostSecretStore;

impl SecretStore for HostSecretStore {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        match secrets_store::get(key) {
            Ok(opt) => Ok(opt),
            Err(err) => Err(format!("secret error: {} - {}", err.name(), err.message())),
        }
    }
}
