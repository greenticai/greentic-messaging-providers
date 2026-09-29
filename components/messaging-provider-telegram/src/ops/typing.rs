//! `send_typing` for Telegram: `sendChatAction` with `action=typing`.
//! Telegram shows it for at most 5 s or until the bot's next message.

use provider_common::typing::{InboundRef, SendTypingOutV1, config_input, parse_send_typing};
use serde_json::{Value, json};

use crate::config::{get_bot_token, load_config};
use crate::{DEFAULT_API_BASE, PROVIDER_TYPE};

use super::http::tg_send_media;

pub(crate) const TELEGRAM_TYPING_REFRESH_MS: u64 = 4_500;

pub(crate) fn send_typing(input_json: &[u8]) -> Vec<u8> {
    match raise(input_json) {
        Ok(()) => SendTypingOutV1::raised(TELEGRAM_TYPING_REFRESH_MS).to_bytes(),
        Err(out) => out.to_bytes(),
    }
}

fn raise(input_json: &[u8]) -> Result<(), SendTypingOutV1> {
    let input = parse_send_typing(input_json, &[PROVIDER_TYPE])?;
    let cfg = load_config(&config_input(&input)).map_err(SendTypingOutV1::failed)?;
    if !cfg.enabled {
        return Err(SendTypingOutV1::failed("provider disabled by config"));
    }
    let (chat_id, thread) = typing_chat(&input.message).map_err(SendTypingOutV1::failed)?;
    let token = get_bot_token(&cfg).map_err(SendTypingOutV1::failed)?;
    let api_base = cfg
        .api_base_url
        .clone()
        .unwrap_or_else(|| DEFAULT_API_BASE.to_string());
    tg_send_media(
        &api_base,
        &token,
        "sendChatAction",
        &chat_action_body(&chat_id, thread),
    )
    .map(|_| ())
    .map_err(|err| SendTypingOutV1::failed(redact_token(&err, &token)))
}

/// `"telegram"` is the ingest fallback for "no chat" and is refused.
fn typing_chat(message: &Value) -> Result<(String, Option<i64>), String> {
    let inbound = InboundRef::new(message);
    let chat_id = inbound
        .first_destination()
        .or_else(|| inbound.metadata("chat_id"))
        .or_else(|| inbound.session_id())
        .filter(|id| *id != "telegram")
        .ok_or_else(|| "inbound envelope carries no Telegram chat".to_string())?;
    let thread = inbound
        .metadata("message_thread_id")
        .and_then(|value| value.parse::<i64>().ok());
    Ok((chat_id.to_string(), thread))
}

pub(crate) fn chat_action_body(chat_id: &str, thread: Option<i64>) -> Value {
    let mut body = json!({"chat_id": chat_id, "action": "typing"});
    if let (Some(thread), Some(map)) = (thread, body.as_object_mut()) {
        map.insert("message_thread_id".into(), json!(thread));
    }
    body
}

/// The bot token is part of every Telegram API URL; a transport error can echo it.
fn redact_token(message: &str, token: &str) -> String {
    if token.is_empty() {
        message.to_string()
    } else {
        message.replace(token, "***")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn body_is_a_typing_chat_action_with_no_text() {
        assert_eq!(
            chat_action_body("777", None),
            json!({"chat_id": "777", "action": "typing"})
        );
        assert_eq!(
            chat_action_body("777", Some(12)),
            json!({"chat_id": "777", "action": "typing", "message_thread_id": 12})
        );
    }

    #[test]
    fn chat_id_prefers_destination_then_metadata_then_session() {
        let full = json!({"to": [{"id": "1"}], "metadata": {"chat_id": "2"}, "session_id": "3"});
        assert_eq!(typing_chat(&full).expect("c").0, "1");
        let meta = json!({"metadata": {"chat_id": "2"}, "session_id": "3"});
        assert_eq!(typing_chat(&meta).expect("c").0, "2");
        assert_eq!(typing_chat(&json!({"session_id": "3"})).expect("c").0, "3");
    }

    #[test]
    fn the_telegram_fallback_session_is_not_a_chat() {
        assert!(typing_chat(&json!({"session_id": "telegram"})).is_err());
        assert!(typing_chat(&json!({})).is_err());
    }

    #[test]
    fn thread_id_is_read_only_when_numeric() {
        let msg = json!({"session_id": "3", "metadata": {"message_thread_id": "12"}});
        assert_eq!(typing_chat(&msg).expect("c").1, Some(12));
        let junk = json!({"session_id": "3", "metadata": {"message_thread_id": "x"}});
        assert_eq!(typing_chat(&junk).expect("c").1, None);
    }

    #[test]
    fn the_bot_token_never_reaches_an_error() {
        let err = redact_token(
            "POST https://api.telegram.org/bot123:ABC/sendChatAction failed",
            "123:ABC",
        );
        assert!(!err.contains("123:ABC"));
    }

    #[test]
    fn bad_input_answers_ok_false() {
        let out: Value = serde_json::from_slice(&send_typing(b"{")).expect("json");
        assert_eq!(out["ok"], false);
    }
}
