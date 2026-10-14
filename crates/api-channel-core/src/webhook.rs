//! Signed webhook delivery with bounded retries.

use crate::config::{ApiConfig, webhook_url_is_acceptable};
use crate::env::{Env, PostOutcome};
use crate::event::Event;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

pub const TIMESTAMP_HEADER: &str = "X-Greentic-Timestamp";
pub const SIGNATURE_HEADER: &str = "X-Greentic-Signature";
/// Total attempts, first try included.
pub const MAX_ATTEMPTS: u32 = 3;
const BACKOFF_BASE_MS: u64 = 200;

/// `sha256=<hex>` of HMAC-SHA256 over `timestamp + "." + body`.
pub fn sign(secret: &str, timestamp: u64, body: &[u8]) -> String {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(secret.as_bytes())
        // HMAC accepts keys of any length; this arm cannot be taken.
        .unwrap_or_else(|_| <Hmac<Sha256> as KeyInit>::new(&Default::default()));
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    let digest = mac.finalize().into_bytes();
    let mut out = String::from("sha256=");
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// No webhook configured; nothing was attempted.
    NotConfigured,
    Delivered,
    /// Gave up (retries exhausted, a 4xx, or an unusable target).
    Failed,
}

/// Push one event. Retries only on retryable outcomes, at most
/// [`MAX_ATTEMPTS`] times, with exponential backoff.
pub fn deliver<E: Env + ?Sized>(env: &mut E, config: &ApiConfig, event: &Event) -> Delivery {
    let (Some(url), Some(secret)) = (&config.webhook_url, &config.webhook_secret) else {
        return Delivery::NotConfigured;
    };
    if !webhook_url_is_acceptable(url) {
        return Delivery::Failed;
    }
    let Ok(body) = serde_json::to_vec(event) else {
        return Delivery::Failed;
    };
    for attempt in 0..MAX_ATTEMPTS {
        // Re-sign every attempt so the timestamp the receiver checks is fresh.
        let timestamp = env.now_unix();
        let headers = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            (TIMESTAMP_HEADER.to_string(), timestamp.to_string()),
            (SIGNATURE_HEADER.to_string(), sign(secret, timestamp, &body)),
        ];
        match env.post(url, &headers, &body) {
            PostOutcome::Delivered => return Delivery::Delivered,
            PostOutcome::Rejected => return Delivery::Failed,
            PostOutcome::Retryable => {
                if attempt + 1 < MAX_ATTEMPTS {
                    env.sleep_ms(BACKOFF_BASE_MS << attempt);
                }
            }
        }
    }
    Delivery::Failed
}
