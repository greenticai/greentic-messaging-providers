use serde::{Deserialize, Serialize};

/// Driver for namespaced state operations used by the Direct Line contract.
pub trait StateStore {
    fn read(&mut self, key: &str) -> Result<Option<Vec<u8>>, String>;
    fn write(&mut self, key: &str, value: &[u8]) -> Result<(), String>;
}

/// Driver for reading secrets required by the Direct Line contract.
pub trait SecretStore {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, String>;
}

/// Simple rate-limit record persisted per (env,tenant,team,user) to enforce token generation limits.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RateLimitState {
    pub window_start: i64,
    pub count: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RateLimitError {
    LimitExceeded,
}

impl RateLimitState {
    pub fn new(now: i64) -> Self {
        RateLimitState {
            window_start: now,
            count: 0,
        }
    }

    pub fn bump(
        &mut self,
        now: i64,
        window_seconds: i64,
        limit: u32,
    ) -> Result<u32, RateLimitError> {
        if now - self.window_start >= window_seconds {
            self.window_start = now;
            self.count = 0;
        }
        if self.count >= limit {
            return Err(RateLimitError::LimitExceeded);
        }
        self.count = self.count.saturating_add(1);
        Ok(self.count)
    }
}
