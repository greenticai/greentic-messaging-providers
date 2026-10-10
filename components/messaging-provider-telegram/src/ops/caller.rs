//! The verified caller block a Telegram turn may carry.
//!
//! greentic-runner reads `extensions.caller` off the envelope this provider
//! emits and trusts it: `user_verified: true` is what the per-end-user ledger,
//! tools and the run audit treat as a provider-verified person. So a block is
//! stamped only when every one of these holds, and never otherwise:
//!
//! 1. **The host authenticated the webhook.** The provider cannot check the
//!    secret token itself (it never sees the secret). greentic-start's
//!    `provider_auth` gate does, and tells us by adding the reserved header
//!    [`VERIFIED_HEADER`] = `telegram` to the `ingest_http` input. The host
//!    strips any client-supplied copy first and adds its own only after the
//!    gate matched a configured `webhook_secret_ref`; a legacy endpoint with no
//!    ref is admitted without the marker. Absent marker: no caller.
//! 2. **The update is a `message` or a `callback_query`** carrying a `from`.
//!    `channel_post` has no `from`; `edited_message` is not handled at all.
//! 3. **The sender is not a bot** (`from.is_bot` is not `true`; a missing flag
//!    is treated as unverifiable and omitted).
//! 4. **The chat is private** (`chat.type == "private"`, for a callback the
//!    chat of the message the button sits on). A person's cross-unit history
//!    must never be injected into a reply a group can read.
//! 5. **`from.id` is a well-formed Telegram user id**: a positive JSON integer,
//!    which in a private chat equals the chat id. Anything else is omitted
//!    rather than coerced.
//!
//! `sub` is the id as a decimal string (stable and unique per account, within
//! the ledger's 256-byte limit); `iss` is the constant `telegram`.

use serde_json::{Map, Value, json};

/// Reserved header the host adds once the webhook secret matched.
pub(crate) const VERIFIED_HEADER: &str = "x-greentic-auth-verified";
/// Its value for a Telegram webhook.
pub(crate) const VERIFIED_VALUE: &str = "telegram";
/// The runner-side envelope extension key.
pub(crate) const CALLER_EXT_KEY: &str = "caller";

const ISSUER: &str = "telegram";
/// Ledger limits on `sub` (bytes) and `iss` (bytes).
const MAX_SUB_BYTES: usize = 256;
const MAX_ISS_BYTES: usize = 512;

/// True iff the host marked this request as authenticated for Telegram.
pub(crate) fn host_verified(input: &Value) -> bool {
    input
        .get("headers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(header_name_and_value)
        .any(|(name, value)| name.eq_ignore_ascii_case(VERIFIED_HEADER) && value == VERIFIED_VALUE)
}

/// Both wire shapes of a header: `[name, value]` and `{name, value}`.
fn header_name_and_value(header: &Value) -> Option<(&str, &str)> {
    match header {
        Value::Array(kv) => Some((kv.first()?.as_str()?, kv.get(1)?.as_str()?)),
        Value::Object(map) => Some((map.get("name")?.as_str()?, map.get("value")?.as_str()?)),
        _ => None,
    }
}

/// The caller block for `update`, or `None` when any condition above fails.
///
/// `host_verified` is the result of [`host_verified`] on the ingest input.
pub(crate) fn verified_caller(update: &Value, host_verified: bool) -> Option<Value> {
    if !host_verified {
        return None;
    }
    // `callback_query` wins in `ingest_http`, so it wins here too.
    let (from, chat) = if let Some(callback) = update.get("callback_query") {
        (callback.get("from")?, callback.get("message")?.get("chat")?)
    } else {
        let message = update.get("message")?;
        (message.get("from")?, message.get("chat")?)
    };
    if from.get("is_bot").and_then(Value::as_bool) != Some(false) {
        return None;
    }
    if chat.get("type").and_then(Value::as_str) != Some("private") {
        return None;
    }
    let id = from
        .get("id")
        .and_then(Value::as_i64)
        .filter(|id| *id > 0)?;
    // In a private chat the chat id IS the user's id; a payload where they
    // differ is not one we vouch for.
    if chat.get("id").and_then(Value::as_i64) != Some(id) {
        return None;
    }
    let sub = id.to_string();
    if sub.len() > MAX_SUB_BYTES || ISSUER.len() > MAX_ISS_BYTES {
        return None;
    }
    let mut block = Map::new();
    block.insert("user_verified".into(), json!(true));
    block.insert("sub".into(), json!(sub));
    block.insert("iss".into(), json!(ISSUER));
    Some(Value::Object(block))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn private_message(id: Value) -> Value {
        json!({"message": {
            "from": {"id": id, "is_bot": false},
            "chat": {"id": id, "type": "private"},
            "text": "hi"
        }})
    }

    fn expected() -> Value {
        json!({"user_verified": true, "sub": "42", "iss": "telegram"})
    }

    #[test]
    fn marker_and_private_message_is_stamped() {
        assert_eq!(
            verified_caller(&private_message(json!(42)), true),
            Some(expected())
        );
    }

    #[test]
    fn no_marker_is_never_stamped() {
        assert_eq!(verified_caller(&private_message(json!(42)), false), None);
    }

    #[test]
    fn group_supergroup_and_channel_are_not_stamped() {
        for kind in ["group", "supergroup", "channel"] {
            let update = json!({"message": {
                "from": {"id": 42, "is_bot": false},
                // Same id on both sides, so only the chat TYPE can refuse it.
                "chat": {"id": 42, "type": kind}
            }});
            assert_eq!(verified_caller(&update, true), None, "{kind}");
        }
    }

    #[test]
    fn missing_chat_type_is_not_stamped() {
        let update = json!({"message": {
            "from": {"id": 42, "is_bot": false}, "chat": {"id": 42}
        }});
        assert_eq!(verified_caller(&update, true), None);
    }

    #[test]
    fn bots_and_unflagged_senders_are_not_stamped() {
        for from in [json!({"id": 42, "is_bot": true}), json!({"id": 42})] {
            let update = json!({"message": {"from": from, "chat": {"id": 42, "type": "private"}}});
            assert_eq!(verified_caller(&update, true), None);
        }
    }

    #[test]
    fn callback_query_in_a_private_chat_is_stamped() {
        let update = json!({"callback_query": {
            "id": "cb", "from": {"id": 42, "is_bot": false},
            "message": {"chat": {"id": 42, "type": "private"}}, "data": "x"
        }});
        assert_eq!(verified_caller(&update, true), Some(expected()));
    }

    #[test]
    fn callback_query_in_a_group_or_without_a_message_is_not_stamped() {
        let group = json!({"callback_query": {
            "from": {"id": 42, "is_bot": false},
            "message": {"chat": {"id": -5, "type": "group"}}
        }});
        let inline = json!({"callback_query": {"from": {"id": 42, "is_bot": false}}});
        assert_eq!(verified_caller(&group, true), None);
        assert_eq!(verified_caller(&inline, true), None);
    }

    #[test]
    fn malformed_ids_are_omitted() {
        for id in [
            json!("42"),
            json!(42.5),
            json!(0),
            json!(-7),
            json!(null),
            json!(true),
        ] {
            assert_eq!(
                verified_caller(&private_message(id.clone()), true),
                None,
                "{id}"
            );
        }
    }

    #[test]
    fn user_and_chat_ids_must_agree() {
        let update = json!({"message": {
            "from": {"id": 42, "is_bot": false}, "chat": {"id": 43, "type": "private"}
        }});
        assert_eq!(verified_caller(&update, true), None);
    }

    #[test]
    fn updates_without_a_from_are_not_stamped() {
        let channel_post = json!({"channel_post": {"chat": {"id": 1, "type": "channel"}}});
        let message = json!({"message": {"chat": {"id": 42, "type": "private"}}});
        assert_eq!(verified_caller(&channel_post, true), None);
        assert_eq!(verified_caller(&message, true), None);
    }

    #[test]
    fn marker_is_read_from_both_header_shapes_case_insensitively() {
        let pairs = json!({"headers": [["X-Greentic-Auth-Verified", "telegram"]]});
        let objects =
            json!({"headers": [{"name": "x-greentic-auth-verified", "value": "telegram"}]});
        assert!(host_verified(&pairs));
        assert!(host_verified(&objects));
    }

    #[test]
    fn a_marker_for_another_value_or_no_headers_is_not_verified() {
        assert!(!host_verified(
            &json!({"headers": [["x-greentic-auth-verified", "slack"]]})
        ));
        assert!(!host_verified(
            &json!({"headers": [["x-greentic-auth-verified", ""]]})
        ));
        assert!(!host_verified(&json!({"headers": []})));
        assert!(!host_verified(&json!({})));
    }
}
