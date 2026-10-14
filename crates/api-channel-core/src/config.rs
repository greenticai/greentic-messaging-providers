//! Channel configuration. Secrets live only in memory and never in `Debug`.

use std::fmt;

#[derive(Clone, Default)]
pub struct ApiConfig {
    /// Bearer key clients present. Absent means every request is refused.
    pub api_key: Option<String>,
    /// Where to push events. Must be `https://`.
    pub webhook_url: Option<String>,
    pub webhook_secret: Option<String>,
}

impl fmt::Debug for ApiConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApiConfig")
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("webhook_url", &self.webhook_url)
            .field(
                "webhook_secret",
                &self.webhook_secret.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

/// A webhook target is usable only when it is an `https://` URL.
pub fn webhook_url_is_acceptable(url: &str) -> bool {
    url.strip_prefix("https://")
        .is_some_and(|rest| !rest.is_empty() && !rest.starts_with('/') && !rest.contains(' '))
}
