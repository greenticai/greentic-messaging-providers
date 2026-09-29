//! `send_typing` for WhatsApp Cloud API: `typing_indicator` on the inbound
//! message. This also marks that message read; the indicator lasts up to 25 s
//! or until the reply.

use provider_common::typing::{InboundRef, SendTypingOutV1, config_input, parse_send_typing};
use serde_json::{Value, json};

use crate::bindings::greentic::http::http_client as client;
use crate::config::{get_token, load_config};
use crate::{DEFAULT_API_BASE, DEFAULT_API_VERSION, PROVIDER_TYPE};

pub(crate) const WHATSAPP_TYPING_REFRESH_MS: u64 = 20_000;

pub(crate) fn send_typing(input_json: &[u8]) -> Vec<u8> {
    match raise(input_json) {
        Ok(()) => SendTypingOutV1::raised(WHATSAPP_TYPING_REFRESH_MS).to_bytes(),
        Err(out) => out.to_bytes(),
    }
}

fn raise(input_json: &[u8]) -> Result<(), SendTypingOutV1> {
    let input = parse_send_typing(input_json, &[PROVIDER_TYPE])?;
    let message_id = inbound_message_id(&input.message).map_err(SendTypingOutV1::failed)?;
    let cfg = load_config(&config_input(&input)).map_err(SendTypingOutV1::failed)?;
    if !cfg.enabled {
        return Err(SendTypingOutV1::failed("provider disabled by config"));
    }
    let token = get_token(&cfg).map_err(SendTypingOutV1::failed)?;
    let api_base = cfg
        .api_base_url
        .clone()
        .unwrap_or_else(|| DEFAULT_API_BASE.to_string());
    let api_version = cfg
        .api_version
        .clone()
        .unwrap_or_else(|| DEFAULT_API_VERSION.to_string());
    let body = serde_json::to_vec(&typing_indicator_body(&message_id))
        .map_err(|err| SendTypingOutV1::failed(err.to_string()))?;
    let request = client::Request {
        method: "POST".into(),
        url: format!("{api_base}/{api_version}/{}/messages", cfg.phone_number_id),
        headers: vec![
            ("Content-Type".into(), "application/json".into()),
            ("Authorization".into(), format!("Bearer {token}")),
        ],
        body: Some(body),
    };
    let resp = client::send(&request, None, None)
        .map_err(|err| SendTypingOutV1::failed(format!("transport error: {}", err.message)))?;
    if (200..300).contains(&resp.status) {
        Ok(())
    } else {
        Err(SendTypingOutV1::failed(format!(
            "whatsapp typing_indicator returned status {}",
            resp.status
        )))
    }
}

fn inbound_message_id(message: &Value) -> Result<String, String> {
    InboundRef::new(message)
        .metadata("wa_message_id")
        .map(str::to_string)
        .ok_or_else(|| "inbound envelope carries no WhatsApp message id".to_string())
}

pub(crate) fn typing_indicator_body(message_id: &str) -> Value {
    json!({
        "messaging_product": "whatsapp",
        "status": "read",
        "message_id": message_id,
        "typing_indicator": {"type": "text"}
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn body_marks_read_and_shows_typing_with_no_text() {
        assert_eq!(
            typing_indicator_body("wamid.ABC"),
            json!({
                "messaging_product": "whatsapp",
                "status": "read",
                "message_id": "wamid.ABC",
                "typing_indicator": {"type": "text"}
            })
        );
    }

    #[test]
    fn needs_the_stamped_inbound_message_id() {
        let msg = json!({"metadata": {"wa_message_id": "wamid.ABC"}});
        assert_eq!(inbound_message_id(&msg).expect("id"), "wamid.ABC");
        assert!(inbound_message_id(&json!({"session_id": "whatsapp"})).is_err());
    }

    #[test]
    fn an_older_envelope_without_the_id_is_ok_false() {
        let input = json!({"v": 1, "provider_type": "messaging.whatsapp.cloud",
            "message": {"session_id": "whatsapp", "metadata": {"from": "447"}}});
        let out: Value =
            serde_json::from_slice(&send_typing(input.to_string().as_bytes())).expect("json");
        assert_eq!(out["ok"], false);
        assert!(out["error"].as_str().unwrap_or("").contains("message id"));
    }
}
