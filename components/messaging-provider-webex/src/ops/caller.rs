//! The verified-caller block for Webex 1:1 messages.
//!
//! greentic-start only trusts a caller established by a messaging provider, and
//! the platform's per-end-user ledger keys on `extensions.caller.{sub,iss}`.
//! This module decides, in ONE place, when Webex may assert one.
//!
//! A caller is stamped only when every condition holds:
//!
//! 1. the webhook signature was actually verified in this request
//!    ([`Verification::Verified`]): a secret resolved, was non-empty, and the
//!    HMAC matched. A skipped check never counts;
//! 2. the event is `messages.created`;
//! 3. the identity comes from the message object fetched from the Webex API with
//!    the bot token, never from the webhook body;
//! 4. the fetched sender is not a bot;
//! 5. the space is 1:1 (`roomType == "direct"`): a person's private history must
//!    not be injected into a reply a whole group space can read.
//!
//! `sub` is the opaque `personId` verbatim, never the email (emails change and
//! have aliases). Anything that does not fit the ledger's limits is omitted.

use serde_json::{Value, json};

use super::ingest_helpers::{MessageDetails, fetch_action_details, fetch_message_details};
use crate::DEFAULT_TOKEN_KEY;
use crate::config::get_secret_string;

/// Envelope extension key the runner reads (`caller_identity::CALLER_EXT_KEY`).
pub(super) const CALLER_EXT_KEY: &str = "caller";
/// Issuer asserted for Webex identities.
pub(super) const CALLER_ISSUER: &str = "webex";
/// Ledger limit on `sub`, in bytes.
const MAX_SUB_BYTES: usize = 256;

/// What the signature check proved for this request, computed once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verification {
    /// A non-empty secret resolved and the signature matched.
    Verified,
    /// The request was admitted without proof of origin.
    Unverified(NotVerified),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NotVerified {
    /// No webhook secret resolved (or the secret store errored): fail-open.
    NoSecret,
    /// A secret resolved but it is empty, so a matching HMAC proves nothing.
    EmptySecret,
}

impl NotVerified {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            NotVerified::NoSecret => "no webhook secret configured",
            NotVerified::EmptySecret => "webhook secret is empty",
        }
    }
}

/// The Webex API calls the ingest path makes, injectable for tests.
pub(super) trait MessageSource {
    fn token(&self) -> Result<String, String>;
    fn fetch_message(
        &self,
        message_id: &str,
        api_base: &str,
        token: &str,
    ) -> Result<MessageDetails, String>;
    fn fetch_action(&self, action_id: &str, api_base: &str, token: &str) -> Result<Value, String>;
}

pub(super) struct HostMessageSource;

impl MessageSource for HostMessageSource {
    fn token(&self) -> Result<String, String> {
        get_secret_string(DEFAULT_TOKEN_KEY)
    }
    fn fetch_message(
        &self,
        message_id: &str,
        api_base: &str,
        token: &str,
    ) -> Result<MessageDetails, String> {
        fetch_message_details(message_id, api_base, token)
    }
    fn fetch_action(&self, action_id: &str, api_base: &str, token: &str) -> Result<Value, String> {
        fetch_action_details(action_id, api_base, token)
    }
}

/// The caller block for a fetched message, or `None` when any condition fails.
pub(super) fn verified_caller(
    verification: Verification,
    resource: &str,
    event: &str,
    details: &MessageDetails,
) -> Option<Value> {
    if verification != Verification::Verified || resource != "messages" || event != "created" {
        return None;
    }
    if details.room_type.as_deref() != Some("direct") {
        return None;
    }
    // A fetched message always carries its author's email; without one the
    // sender cannot be shown to be a human, so assert nothing.
    let email = details.person_email.as_deref()?;
    if is_bot_email(email) {
        return None;
    }
    let sub = details
        .person_id
        .as_deref()
        .filter(|sub| valid_ledger_text(sub, MAX_SUB_BYTES))?;
    Some(json!({"user_verified": true, "sub": sub, "iss": CALLER_ISSUER}))
}

fn is_bot_email(value: &str) -> bool {
    value.trim().to_ascii_lowercase().ends_with("@webex.bot")
}

/// Non-empty, within `max` bytes, no control characters, no surrounding
/// whitespace: the shape the ledger accepts verbatim.
fn valid_ledger_text(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value == value.trim()
        && !value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn details(room_type: Option<&str>, email: Option<&str>, id: Option<&str>) -> MessageDetails {
        MessageDetails {
            markdown: None,
            text: Some("hi".into()),
            room_id: Some("room-1".into()),
            person_email: email.map(str::to_string),
            person_id: id.map(str::to_string),
            room_type: room_type.map(str::to_string),
            attachments: Vec::new(),
        }
    }

    fn stamp(v: Verification, res: &str, ev: &str, d: &MessageDetails) -> Option<Value> {
        verified_caller(v, res, ev, d)
    }

    #[test]
    fn a_verified_direct_message_stamps_person_id_and_webex_issuer() {
        let d = details(
            Some("direct"),
            Some("ada@example.com"),
            Some("Y2lzY29zcGFyazovL3BlcnNvbi8x"),
        );
        assert_eq!(
            stamp(Verification::Verified, "messages", "created", &d),
            Some(
                json!({"user_verified": true, "sub": "Y2lzY29zcGFyazovL3BlcnNvbi8x", "iss": "webex"})
            )
        );
    }

    #[test]
    fn an_unverified_request_never_stamps() {
        let d = details(Some("direct"), Some("ada@example.com"), Some("p1"));
        for reason in [NotVerified::NoSecret, NotVerified::EmptySecret] {
            assert!(stamp(Verification::Unverified(reason), "messages", "created", &d).is_none());
        }
    }

    #[test]
    fn group_and_unknown_spaces_never_stamp() {
        for room in [Some("group"), Some("Direct"), Some(""), None] {
            let d = details(room, Some("ada@example.com"), Some("p1"));
            assert!(
                stamp(Verification::Verified, "messages", "created", &d).is_none(),
                "{room:?}"
            );
        }
    }

    #[test]
    fn bot_senders_never_stamp() {
        let d = details(Some("direct"), Some("Greentic_CI@WEBEX.BOT"), Some("p1"));
        assert!(stamp(Verification::Verified, "messages", "created", &d).is_none());
        let d = details(Some("direct"), None, Some("p1"));
        assert!(stamp(Verification::Verified, "messages", "created", &d).is_none());
    }

    #[test]
    fn other_resources_and_events_never_stamp() {
        let d = details(Some("direct"), Some("ada@example.com"), Some("p1"));
        for (r, e) in [
            ("memberships", "created"),
            ("attachmentActions", "created"),
            ("messages", "deleted"),
            ("messages", "updated"),
        ] {
            assert!(stamp(Verification::Verified, r, e, &d).is_none(), "{r}.{e}");
        }
    }

    #[test]
    fn the_email_is_never_the_sub_and_a_missing_person_id_omits_the_block() {
        let d = details(Some("direct"), Some("ada@example.com"), None);
        assert!(stamp(Verification::Verified, "messages", "created", &d).is_none());
    }

    #[test]
    fn malformed_person_ids_are_omitted() {
        let too_long = "a".repeat(257);
        for bad in ["", " p1", "p1 ", "p\u{0}1", "p\n1", too_long.as_str()] {
            let d = details(Some("direct"), Some("ada@example.com"), Some(bad));
            assert!(
                stamp(Verification::Verified, "messages", "created", &d).is_none(),
                "{bad:?}"
            );
        }
        let at_limit = "a".repeat(256);
        let d = details(Some("direct"), Some("ada@example.com"), Some(&at_limit));
        assert!(stamp(Verification::Verified, "messages", "created", &d).is_some());
    }
}
