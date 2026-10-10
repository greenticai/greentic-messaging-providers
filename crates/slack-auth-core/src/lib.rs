//! Slack request authentication and verified-caller derivation.
//!
//! One implementation of Slack's `v0` request signature check, shared by
//! `messaging-ingress-slack` (the component greentic-start's webhook gate
//! runs) and `messaging-provider-slack` (whose `ingest_http` stamps a verified
//! caller). Two copies of a signature check drift; one cannot.
//!
//! The crate is pure: it never reads a clock or a secret. Callers pass the
//! current time in, so the replay window is testable with a fixed clock and
//! the crate compiles identically for native tests and for `wasm32-wasip2`.
//!
//! ## What a verified caller is
//!
//! [`caller_for_event`], [`caller_for_block_actions`] and
//! [`caller_for_view_submission`] return the `extensions.caller` block
//! `{"user_verified": true, "sub": <slack user id>, "iss": "slack:<team>"}`
//! the runner turns into a per-end-user ledger key. They must ONLY be called
//! for a request whose signature [`verify_request`] accepted: a Slack user id
//! in an unauthenticated body is attacker-chosen text.
//!
//! Privacy rule: a caller is derived for a 1:1 conversation (an `im` / `D…`
//! channel) and nowhere else. In a channel, private group or multi-party DM
//! the reply is readable by everyone present, and a person's private
//! cross-unit history must not be injected into it.

use hmac::{Hmac, KeyInit, Mac};
use serde_json::{Value, json};
use sha2::Sha256;

/// Slack's documented replay window: a request whose timestamp differs from
/// now by more than this many seconds, in either direction, is refused.
pub const MAX_SKEW_SECS: i64 = 300;

/// The ledger's limit for `sub` (bytes).
pub const MAX_SUB_BYTES: usize = 256;
/// The ledger's limit for `iss` (bytes).
pub const MAX_ISS_BYTES: usize = 512;

/// Why a request was not authenticated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyError {
    /// The signing secret is empty.
    InvalidSecret,
    /// `x-slack-request-timestamp` is not a decimal number of seconds.
    InvalidTimestamp,
    /// The timestamp is more than [`MAX_SKEW_SECS`] in the past.
    StaleTimestamp,
    /// The timestamp is more than [`MAX_SKEW_SECS`] in the future.
    FutureTimestamp,
    /// The signature does not match.
    InvalidSignature,
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            VerifyError::InvalidSecret => "invalid secret",
            VerifyError::InvalidTimestamp => "invalid timestamp",
            VerifyError::StaleTimestamp => "stale timestamp",
            VerifyError::FutureTimestamp => "timestamp in the future",
            VerifyError::InvalidSignature => "invalid signature",
        })
    }
}

impl std::error::Error for VerifyError {}

/// Seconds since the Unix epoch according to the system clock.
///
/// On `wasm32-wasip2` this is `wasi:clocks/wall-clock`. A clock before the
/// epoch reads as 0, which makes every real request stale: the failure is a
/// refusal, never an acceptance.
pub fn now_unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Compare two byte strings without an early exit on the first difference.
///
/// Length is not secret (a Slack signature has a fixed length), so unequal
/// lengths return at once.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Verify Slack's `v0` request signature over the RAW body bytes.
///
/// `signature` is the `X-Slack-Signature` header (`v0=<hex>`), `timestamp`
/// the `X-Slack-Request-Timestamp` header and `now_secs` the current time.
/// The timestamp window is checked before the HMAC so a replayed capture is
/// refused whatever its signature.
pub fn verify_request(
    secret: &str,
    signature: &str,
    timestamp: &str,
    body: &[u8],
    now_secs: i64,
) -> Result<(), VerifyError> {
    if secret.is_empty() {
        return Err(VerifyError::InvalidSecret);
    }
    let ts: i64 = timestamp
        .trim()
        .parse()
        .map_err(|_| VerifyError::InvalidTimestamp)?;
    let skew = now_secs.saturating_sub(ts);
    if skew > MAX_SKEW_SECS {
        return Err(VerifyError::StaleTimestamp);
    }
    if skew < -MAX_SKEW_SECS {
        return Err(VerifyError::FutureTimestamp);
    }

    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .map_err(|_| VerifyError::InvalidSecret)?;
    mac.update(b"v0:");
    mac.update(timestamp.as_bytes());
    mac.update(b":");
    mac.update(body);
    let computed = format!("v0={}", hex_encode(&mac.finalize().into_bytes()));

    if constant_time_eq(computed.as_bytes(), signature.as_bytes()) {
        Ok(())
    } else {
        Err(VerifyError::InvalidSignature)
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

// ---------------------------------------------------------------------------
// Verified caller
// ---------------------------------------------------------------------------

/// A Slack user id: `U…` or `W…` followed by upper-case alphanumerics.
fn valid_user_id(id: &str) -> bool {
    valid_slack_id(id, &['U', 'W'])
}

/// A workspace (`T…`) or enterprise (`E…`) id.
fn valid_org_id(id: &str) -> bool {
    valid_slack_id(id, &['T', 'E'])
}

fn valid_slack_id(id: &str, prefixes: &[char]) -> bool {
    let mut chars = id.chars();
    match chars.next() {
        Some(first) if prefixes.contains(&first) => {}
        _ => return false,
    }
    let rest = chars.as_str();
    !rest.is_empty()
        && id.len() <= 64
        && rest
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// The issuer's organisation part.
///
/// - the user's own team differs from the workspace the event arrived in
///   (Slack Connect, or another workspace of the org): the user's own team,
///   which together with the id is unambiguous;
/// - otherwise the enterprise id on Enterprise Grid, where ids are unique
///   across the whole org;
/// - otherwise the workspace id.
fn issuer_part<'a>(
    team: Option<&'a str>,
    enterprise: Option<&'a str>,
    user_team: Option<&'a str>,
) -> Option<&'a str> {
    let team = team.filter(|v| valid_org_id(v));
    let enterprise = enterprise.filter(|v| valid_org_id(v));
    let user_team = user_team.filter(|v| valid_org_id(v));
    match (user_team, team) {
        (Some(ut), Some(t)) if ut != t => Some(ut),
        _ => enterprise.or(team).or(user_team),
    }
}

/// Build the caller block. `None` when `user` or the issuer is malformed.
pub fn caller_block(
    user: &str,
    team: Option<&str>,
    enterprise: Option<&str>,
    user_team: Option<&str>,
) -> Option<Value> {
    if !valid_user_id(user) {
        return None;
    }
    let part = issuer_part(team, enterprise, user_team)?;
    let iss = format!("slack:{part}");
    if user.len() > MAX_SUB_BYTES || iss.len() > MAX_ISS_BYTES {
        return None;
    }
    Some(json!({"user_verified": true, "sub": user, "iss": iss}))
}

fn str_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut cur = value;
    for key in path {
        cur = cur.get(*key)?;
    }
    cur.as_str().filter(|s| !s.is_empty())
}

/// A 1:1 conversation: `channel_type == "im"`, or — when Slack sent no type —
/// a `D…` channel id. A present type that is anything else (`channel`,
/// `group`, `mpim`) is refused even if the id looks like a DM.
fn is_direct_message(channel: Option<&str>, channel_type: Option<&str>) -> bool {
    let Some(channel) = channel else {
        return false;
    };
    match channel_type {
        Some("im") => true,
        Some(_) => false,
        None => channel.starts_with('D'),
    }
}

fn enterprise_of(body: &Value) -> Option<&str> {
    str_at(body, &["enterprise_id"])
        .or_else(|| str_at(body, &["enterprise", "id"]))
        .or_else(|| {
            body.get("authorizations")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(|a| str_at(a, &["enterprise_id"]))
        })
}

fn team_of(body: &Value) -> Option<&str> {
    str_at(body, &["team_id"])
        .or_else(|| str_at(body, &["team", "id"]))
        .or_else(|| {
            body.get("authorizations")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(|a| str_at(a, &["team_id"]))
        })
}

/// Caller for an Events API `event_callback` carrying a direct message.
///
/// `body` is the whole callback, `event` its `event` object. `None` for a
/// bot or app event, an event without a user, an edit or deletion, a
/// non-message event, and anything outside a 1:1 conversation.
pub fn caller_for_event(body: &Value) -> Option<Value> {
    if str_at(body, &["type"]) != Some("event_callback") {
        return None;
    }
    let event = body.get("event")?;
    if str_at(event, &["type"]) != Some("message") {
        return None;
    }
    if event.get("bot_id").is_some_and(|v| !v.is_null())
        || event.get("bot_profile").is_some_and(|v| !v.is_null())
    {
        return None;
    }
    if matches!(
        str_at(event, &["subtype"]),
        Some("bot_message" | "message_changed" | "message_deleted")
    ) {
        return None;
    }
    if !is_direct_message(
        str_at(event, &["channel"]),
        str_at(event, &["channel_type"]),
    ) {
        return None;
    }
    let user = event.get("user")?.as_str()?;
    caller_block(
        user,
        team_of(body),
        enterprise_of(body),
        str_at(event, &["user_team"]),
    )
}

/// Caller for a `block_actions` interaction payload (button click).
pub fn caller_for_block_actions(payload: &Value) -> Option<Value> {
    if str_at(payload, &["type"]) != Some("block_actions") {
        return None;
    }
    let channel = str_at(payload, &["channel", "id"])
        .or_else(|| str_at(payload, &["container", "channel_id"]));
    interaction_caller(payload, channel)
}

/// Caller for a `view_submission`. A modal has no channel of its own: the
/// provider preserves the originating one in `private_metadata`, and the
/// caller passes it as `origin_channel`.
pub fn caller_for_view_submission(payload: &Value, origin_channel: Option<&str>) -> Option<Value> {
    if str_at(payload, &["type"]) != Some("view_submission") {
        return None;
    }
    interaction_caller(payload, origin_channel.filter(|c| !c.is_empty()))
}

fn interaction_caller(payload: &Value, channel: Option<&str>) -> Option<Value> {
    // Interaction payloads carry no channel type; the id prefix decides.
    if !is_direct_message(channel, None) {
        return None;
    }
    let user = payload.get("user")?;
    caller_block(
        user.get("id")?.as_str()?,
        team_of(payload),
        enterprise_of(payload),
        str_at(user, &["team_id"]),
    )
}

/// The runner-side envelope extension key the caller block is stamped under.
pub const CALLER_EXT_KEY: &str = "caller";

#[cfg(test)]
mod tests;
