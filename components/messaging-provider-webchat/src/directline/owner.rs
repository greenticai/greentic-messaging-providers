// One ownership decision for every conversation-scoped Direct Line route.

use greentic_types::messaging::universal_dto::HttpOutV1;

use super::http::{respond_error, respond_forbidden_coded, respond_not_found};
use super::jwt::TokenClaims;
use super::state::{ConversationState, conversation_key};
use super::store::StateStore;

pub const OWNER_REQUIRED_CODE: &str = "ConversationOwnerRequired";
pub const OWNER_REQUIRED_MESSAGE: &str =
    "this conversation belongs to another session; start a new conversation";

pub enum Lookup {
    Found(ConversationState),
    Missing,
}

#[derive(Debug)]
pub enum Authorized {
    /// The token's signed `conv` names this conversation.
    Bound(ConversationState),
    /// Conversation-less verified token matching a verified owner.
    VerifiedOwner(ConversationState),
}

impl Authorized {
    pub fn into_conversation(self) -> ConversationState {
        match self {
            Authorized::Bound(state) | Authorized::VerifiedOwner(state) => state,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    WrongConversation,
    NotFound,
    TokenContextMismatch,
    OwnerRequired,
}

impl Refusal {
    pub fn into_response(self) -> HttpOutV1 {
        match self {
            Refusal::WrongConversation => respond_forbidden_coded(
                "WrongConversation",
                "token bound to different conversation",
            ),
            Refusal::NotFound => respond_not_found("conversation not found"),
            Refusal::TokenContextMismatch => {
                respond_forbidden_coded("TokenContextMismatch", "token context mismatch")
            }
            Refusal::OwnerRequired => {
                respond_forbidden_coded(OWNER_REQUIRED_CODE, OWNER_REQUIRED_MESSAGE)
            }
        }
    }
}

pub fn authorize(
    claims: &TokenClaims,
    conversation_id: &str,
    lookup: Lookup,
) -> Result<Authorized, Refusal> {
    match claims.conv.as_deref() {
        Some(bound) if bound != conversation_id => Err(Refusal::WrongConversation),
        Some(_) => match lookup {
            Lookup::Missing => Err(Refusal::NotFound),
            Lookup::Found(state) if state.ctx != claims.ctx => Err(Refusal::TokenContextMismatch),
            Lookup::Found(state) => Ok(Authorized::Bound(state)),
        },
        // Every unbound refusal is the same answer, so an unbound token is no existence oracle.
        None => match lookup {
            Lookup::Found(state) if is_verified_owner(claims, &state) => {
                Ok(Authorized::VerifiedOwner(state))
            }
            _ => Err(Refusal::OwnerRequired),
        },
    }
}

/// Loads the conversation under the token's own context and authorizes it,
/// returning its state key and header.
pub fn authorize_stored<S: StateStore>(
    store: &mut S,
    claims: &TokenClaims,
    conversation_id: &str,
) -> Result<(String, ConversationState), HttpOutV1> {
    let conv_key = conversation_key(&claims.ctx, conversation_id);
    let lookup = match store.read(&conv_key) {
        Ok(Some(bytes)) => match serde_json::from_slice(&bytes) {
            Ok(state) => Lookup::Found(state),
            // An unreadable header must not tell an unbound caller the id exists.
            Err(_) if claims.conv.is_none() => Lookup::Missing,
            Err(err) => return Err(respond_error(500, "state_parse", err.to_string())),
        },
        Ok(None) => Lookup::Missing,
        Err(err) => return Err(respond_error(500, "state_read", err)),
    };
    authorize(claims, conversation_id, lookup)
        .map(|authorized| (conv_key, authorized.into_conversation()))
        .map_err(Refusal::into_response)
}

fn is_verified_owner(claims: &TokenClaims, state: &ConversationState) -> bool {
    claims.verified
        && state.owner_verified
        && state.owner_sub.as_deref() == Some(claims.sub.as_str())
        && state.ctx == claims.ctx
}

#[cfg(test)]
mod tests {
    include!("owner_tests.rs");
}
