//! The `ingest_http` op: `POST /messages` (send) and `GET /messages` (poll).

use crate::auth::bearer_ok;
use crate::config::ApiConfig;
use crate::env::Env;
use crate::http::{HttpRequest, HttpResponse, Inbound, IngestResult};
use crate::log::{self, LogError};
use crate::store::StateStore;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// Largest accepted request body.
pub const MAX_BODY_BYTES: usize = 64 * 1024;
const MAX_TEXT_BYTES: usize = 32 * 1024;
const MAX_ID_LEN: usize = 64;

#[derive(Deserialize)]
struct PostBody {
    #[serde(default)]
    conversation_id: Option<String>,
    #[serde(default)]
    client_message_id: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    card: Option<Value>,
}

pub fn ingest_http<E: Env + ?Sized>(
    store: &mut dyn StateStore,
    env: &mut E,
    config: &ApiConfig,
    req: &HttpRequest,
) -> IngestResult {
    // Authentication comes first, before the body is even parsed, so an
    // unauthenticated caller learns nothing and touches no state.
    if !bearer_ok(req, config.api_key.as_deref()) {
        return IngestResult::reply(HttpResponse::error(401, "unauthorized"));
    }
    if !req.path.trim_end_matches('/').ends_with("/messages") {
        return IngestResult::reply(HttpResponse::error(404, "not_found"));
    }
    match req.method.to_ascii_uppercase().as_str() {
        "POST" => post_message(store, env, req),
        "GET" => get_messages(store, req),
        _ => IngestResult::reply(HttpResponse::error(405, "method_not_allowed")),
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_LEN
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn message_id(conversation: &str, client_message_id: &str) -> String {
    let digest = Sha256::digest(format!("{conversation}\u{1f}{client_message_id}").as_bytes());
    let hex: String = digest.iter().take(16).map(|b| format!("{b:02x}")).collect();
    format!("m_{hex}")
}

fn dedup_key(conversation: &str, client_message_id: &str) -> String {
    let digest = Sha256::digest(client_message_id.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("api/v1/{conversation}/msg/{hex}")
}

fn store_failure(err: &LogError) -> IngestResult {
    let status = match err {
        LogError::ConversationNotFound => 404,
        LogError::CursorExpired { .. } => 410,
        LogError::Unsupported | LogError::Store(_) => 500,
    };
    IngestResult::reply(HttpResponse::json(status, &log::log_error_value(err)))
}

fn post_message<E: Env + ?Sized>(
    store: &mut dyn StateStore,
    env: &mut E,
    req: &HttpRequest,
) -> IngestResult {
    if req.body.len() > MAX_BODY_BYTES {
        return IngestResult::reply(HttpResponse::error(413, "payload_too_large"));
    }
    let Ok(body) = serde_json::from_slice::<PostBody>(&req.body) else {
        return IngestResult::reply(HttpResponse::error(400, "invalid_json"));
    };
    let Some(client_message_id) = body
        .client_message_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty() && id.len() <= 128)
    else {
        return IngestResult::reply(HttpResponse::error(400, "client_message_id_required"));
    };
    let text = body.text.filter(|t| !t.is_empty());
    if text.is_none() && body.card.is_none() {
        return IngestResult::reply(HttpResponse::error(400, "text_or_card_required"));
    }
    if text.as_ref().is_some_and(|t| t.len() > MAX_TEXT_BYTES) {
        return IngestResult::reply(HttpResponse::error(413, "payload_too_large"));
    }
    let conversation = match body.conversation_id {
        Some(id) if valid_id(&id) => id,
        Some(_) => return IngestResult::reply(HttpResponse::error(400, "invalid_conversation_id")),
        None => format!("c_{}", env.random_id()),
    };
    if let Err(err) = log::ensure_conversation(store, &conversation) {
        return store_failure(&err);
    }
    let message_id = message_id(&conversation, client_message_id);
    let accepted = json!({"conversation_id": conversation, "message_id": message_id});
    // A retry of the same client_message_id answers identically and starts no
    // second turn: the dedup marker is claimed atomically.
    let first = match store.write_if_absent(
        &dedup_key(&conversation, client_message_id),
        message_id.as_bytes(),
    ) {
        Ok(first) => first,
        Err(err) => return store_failure(&err.into()),
    };
    if !first {
        return IngestResult::reply(HttpResponse::json(202, &accepted));
    }
    IngestResult {
        response: HttpResponse::json(202, &accepted),
        inbound: Some(Inbound {
            conversation_id: conversation,
            message_id,
            text,
            card: body.card,
        }),
    }
}

fn get_messages(_store: &mut dyn StateStore, _req: &HttpRequest) -> IngestResult {
    IngestResult::reply(HttpResponse::error(501, "not_implemented"))
}
