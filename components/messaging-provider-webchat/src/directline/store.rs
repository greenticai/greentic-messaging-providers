use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::bindings::greentic::secrets_store::secrets_store;

/// Driver for namespaced state operations used by the Direct Line contract.
pub trait StateStore {
    fn read(&mut self, key: &str) -> Result<Option<Vec<u8>>, String>;
    fn write(&mut self, key: &str, value: &[u8]) -> Result<(), String>;
}

/// Driver for reading secrets required by the Direct Line contract.
pub trait SecretStore {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, String>;
}

/// Host-backed state store implementation.
pub struct HostStateStore;

fn state_map() -> &'static Mutex<HashMap<String, Vec<u8>>> {
    static STATE: OnceLock<Mutex<HashMap<String, Vec<u8>>>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(HashMap::new()))
}

impl StateStore for HostStateStore {
    fn read(&mut self, key: &str) -> Result<Option<Vec<u8>>, String> {
        let guard = state_map()
            .lock()
            .map_err(|_| "state read error: lock poisoned".to_string())?;
        Ok(guard.get(key).cloned())
    }

    fn write(&mut self, key: &str, value: &[u8]) -> Result<(), String> {
        let mut guard = state_map()
            .lock()
            .map_err(|_| "state write error: lock poisoned".to_string())?;
        guard.insert(key.to_string(), value.to_vec());
        Ok(())
    }
}

/// Host-backed secrets drive implementation.
pub struct HostSecretStore;

impl SecretStore for HostSecretStore {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        match secrets_store::get(key) {
            Ok(opt) => Ok(opt),
            Err(err) => Err(format!("secret error: {} - {}", err.name(), err.message())),
        }
    }
}

/// Simple rate-limit record persisted per (env,tenant,team,user) to enforce token generation limits.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RateLimitState {
    pub window_start: i64,
    pub count: u32,
}

impl RateLimitState {
    pub fn new(now: i64) -> Self {
        RateLimitState {
            window_start: now,
            count: 0,
        }
    }

    pub fn bump(&mut self, now: i64, window_seconds: i64, limit: u32) -> Result<u32, ()> {
        if now - self.window_start >= window_seconds {
            self.window_start = now;
            self.count = 0;
        }
        if self.count >= limit {
            return Err(());
        }
        self.count = self.count.saturating_add(1);
        Ok(self.count)
    }
}
