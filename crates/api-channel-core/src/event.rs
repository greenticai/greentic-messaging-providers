//! The cross-repo event schema (see `docs/fixtures/api-channel/`).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What kind of thing happened in the conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    Message,
    InputRequired,
    Error,
    Done,
    DeliveryFailed,
}

/// One piece of an event's content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Item {
    Text { text: String },
    Card { card: Value, fallback: String },
}

/// A conversation event. Field order is part of the contract: fixtures are
/// compared byte for byte.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub cursor: u64,
    #[serde(rename = "type")]
    pub kind: EventType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub items: Vec<Item>,
    pub ts: String,
}

/// An event before the log assigns it a cursor and a timestamp.
#[derive(Debug, Clone, PartialEq)]
pub struct NewEvent {
    pub kind: EventType,
    pub code: Option<String>,
    pub items: Vec<Item>,
}

impl NewEvent {
    pub fn message(items: Vec<Item>) -> Self {
        Self {
            kind: EventType::Message,
            code: None,
            items,
        }
    }

    pub fn of(kind: EventType) -> Self {
        Self {
            kind,
            code: None,
            items: Vec::new(),
        }
    }

    pub fn error(code: impl Into<String>) -> Self {
        Self {
            kind: EventType::Error,
            code: Some(code.into()),
            items: Vec::new(),
        }
    }
}
