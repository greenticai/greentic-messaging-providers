//! The per-conversation event log.
//!
//! Layout, per conversation `<c>` (all under `api/v1/<c>/`):
//!
//! | key        | value                                             |
//! |------------|---------------------------------------------------|
//! | `meta`     | marker that the conversation exists               |
//! | `ev/<n>`   | the event with cursor `n` (JSON), `n` starts at 1 |
//! | `head`     | hint: the last cursor some appender claimed       |
//! | `oldest`   | hint: the oldest cursor still retained            |
//!
//! A cursor is claimed with the atomic `write-if-absent` on `ev/<n>`, so two
//! concurrent appenders can never share one. `head` and `oldest` are only
//! hints that save probing; a stale hint costs probes, never correctness,
//! because an appender only ever advances `head` past a slot it just claimed
//! and every lower slot was claimed before it.

use crate::env::Env;
use crate::event::{Event, NewEvent};
use crate::store::{StateStore, StoreError};
use serde::{Deserialize, Serialize};

/// How many events a conversation keeps. Older cursors answer "expired".
pub const RETAINED_EVENTS: u64 = 1000;
/// Most events a single poll returns.
pub const POLL_LIMIT: u64 = 100;
/// Most slots an append probes past its hint before giving up.
const MAX_PROBE: u64 = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogError {
    ConversationNotFound,
    /// `since` is older than the retained log.
    CursorExpired {
        oldest_cursor: u64,
    },
    /// The host state store cannot create a key atomically.
    Unsupported,
    /// Host failure; the text is for server logs only.
    Store(String),
}

impl From<StoreError> for LogError {
    fn from(err: StoreError) -> Self {
        match err {
            StoreError::Unsupported => LogError::Unsupported,
            StoreError::Failed(text) => LogError::Store(text),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PollOutcome {
    pub events: Vec<Event>,
    pub next_cursor: u64,
}

fn key(conversation: &str, tail: &str) -> String {
    format!("api/v1/{conversation}/{tail}")
}

fn read_u64(store: &mut dyn StateStore, key: &str) -> Result<Option<u64>, LogError> {
    Ok(store
        .read(key)?
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .and_then(|text| text.trim().parse::<u64>().ok()))
}

/// Create the conversation marker if it is absent. Returns whether it was new.
pub fn ensure_conversation(
    store: &mut dyn StateStore,
    conversation: &str,
) -> Result<bool, LogError> {
    Ok(store.write_if_absent(&key(conversation, "meta"), b"1")?)
}

pub fn conversation_exists(
    store: &mut dyn StateStore,
    conversation: &str,
) -> Result<bool, LogError> {
    Ok(store.read(&key(conversation, "meta"))?.is_some())
}

/// Append an event, assigning the next free cursor and the current time.
pub fn append<E: Env + ?Sized>(
    store: &mut dyn StateStore,
    env: &E,
    conversation: &str,
    new: NewEvent,
) -> Result<Event, LogError> {
    if !conversation_exists(store, conversation)? {
        return Err(LogError::ConversationNotFound);
    }
    let start = read_u64(store, &key(conversation, "head"))?.unwrap_or(0) + 1;
    let ts = env.now_rfc3339();
    for cursor in start..start + MAX_PROBE {
        let event = Event {
            cursor,
            kind: new.kind,
            code: new.code.clone(),
            items: new.items.clone(),
            ts: ts.clone(),
        };
        let bytes = serde_json::to_vec(&event).map_err(|e| LogError::Store(e.to_string()))?;
        if store.write_if_absent(&key(conversation, &format!("ev/{cursor}")), &bytes)? {
            store.write(&key(conversation, "head"), cursor.to_string().as_bytes())?;
            trim(store, conversation, cursor)?;
            return Ok(event);
        }
    }
    Err(LogError::Store("no free cursor within probe window".into()))
}

/// Drop the slot that just left the retention window and advance `oldest`.
fn trim(store: &mut dyn StateStore, conversation: &str, claimed: u64) -> Result<(), LogError> {
    if claimed <= RETAINED_EVENTS {
        return Ok(());
    }
    let expired = claimed - RETAINED_EVENTS;
    store.delete(&key(conversation, &format!("ev/{expired}")))?;
    let current = read_u64(store, &key(conversation, "oldest"))?.unwrap_or(1);
    let oldest = current.max(expired + 1);
    store.write(&key(conversation, "oldest"), oldest.to_string().as_bytes())?;
    Ok(())
}

/// Events with `cursor >= since`, ascending, at most [`POLL_LIMIT`].
pub fn poll(
    store: &mut dyn StateStore,
    conversation: &str,
    since: u64,
) -> Result<PollOutcome, LogError> {
    if !conversation_exists(store, conversation)? {
        return Err(LogError::ConversationNotFound);
    }
    let oldest = read_u64(store, &key(conversation, "oldest"))?.unwrap_or(1);
    if since != 0 && since < oldest {
        return Err(LogError::CursorExpired {
            oldest_cursor: oldest,
        });
    }
    let start = since.max(oldest);
    let mut events = Vec::new();
    let mut cursor = start;
    while cursor < start + POLL_LIMIT {
        let Some(bytes) = store.read(&key(conversation, &format!("ev/{cursor}")))? else {
            break;
        };
        match serde_json::from_slice::<Event>(&bytes) {
            Ok(event) => events.push(event),
            Err(err) => return Err(LogError::Store(format!("corrupt event {cursor}: {err}"))),
        }
        cursor += 1;
    }
    // `cursor` is now the first slot not returned, which is exactly what the
    // caller passes as `since` next time.
    Ok(PollOutcome {
        events,
        next_cursor: cursor,
    })
}

#[derive(Deserialize)]
struct PollRequest {
    conversation_id: String,
    #[serde(default)]
    since: u64,
}

/// The `poll_events` provider op: JSON in, JSON out.
///
/// Success is `{"events":[..],"next_cursor":n}`. Failures are
/// `{"error":{"code":..}}` with `cursor_expired` also carrying
/// `oldest_cursor`; no host text ever reaches the output.
pub fn run_poll_events(store: &mut dyn StateStore, input: &[u8]) -> Vec<u8> {
    let value = match serde_json::from_slice::<PollRequest>(input) {
        Err(_) => error_json_value("invalid_request", None),
        Ok(req) => match poll(store, &req.conversation_id, req.since) {
            // Serialize the typed value directly so field order stays the
            // one the shared fixtures pin.
            Ok(out) => return serde_json::to_vec(&out).unwrap_or_default(),
            Err(err) => log_error_value(&err),
        },
    };
    serde_json::to_vec(&value).unwrap_or_default()
}

fn error_json_value(code: &str, oldest: Option<u64>) -> serde_json::Value {
    let mut error = serde_json::json!({ "code": code });
    if let (Some(oldest), Some(map)) = (oldest, error.as_object_mut()) {
        map.insert("oldest_cursor".into(), oldest.into());
    }
    serde_json::json!({ "error": error })
}

/// Map a log failure to its wire shape. Shared with the HTTP surface.
pub fn log_error_value(err: &LogError) -> serde_json::Value {
    match err {
        LogError::ConversationNotFound => error_json_value("conversation_not_found", None),
        LogError::CursorExpired { oldest_cursor } => {
            error_json_value("cursor_expired", Some(*oldest_cursor))
        }
        LogError::Unsupported => error_json_value("state_unsupported", None),
        LogError::Store(_) => error_json_value("internal_error", None),
    }
}
