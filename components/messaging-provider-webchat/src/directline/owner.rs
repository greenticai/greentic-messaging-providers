// One ownership decision for every conversation-scoped Direct Line route.

use greentic_types::messaging::universal_dto::HttpOutV1;

use super::http::{respond_forbidden_coded, respond_not_found};
use super::jwt::TokenClaims;
use super::state::ConversationState;

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
