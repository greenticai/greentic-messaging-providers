//! `send_typing` for the Teams Bot Framework pack: a `{"type":"typing"}`
//! activity on the connector path replies use. Graph mode has no typing API,
//! so the Graph pack does not advertise the op and this refuses it.

use provider_common::typing::{InboundRef, SendTypingOutV1, config_input, parse_send_typing};
use serde_json::{Value, json};

use crate::auth::acquire_bot_token;
use crate::bindings::greentic::http::http_client as client;
use crate::config::load_config;

use super::connector::connector_url;

pub(crate) const TEAMS_TYPING_REFRESH_MS: u64 = 3_000;

const ACCEPTED_PROVIDER_TYPES: &[&str] = &["messaging.teams", crate::PROVIDER_TYPE];

pub(crate) fn send_typing(input_json: &[u8]) -> Vec<u8> {
    match raise(input_json) {
        Ok(()) => SendTypingOutV1::raised(TEAMS_TYPING_REFRESH_MS).to_bytes(),
        Err(out) => out.to_bytes(),
    }
}

fn raise(input_json: &[u8]) -> Result<(), SendTypingOutV1> {
    let input = parse_send_typing(input_json, ACCEPTED_PROVIDER_TYPES)?;
    let cfg = load_config(&config_input(&input)).map_err(SendTypingOutV1::failed)?;
    if !cfg.enabled {
        return Err(SendTypingOutV1::failed("provider disabled by config"));
    }
    let (service_url, conversation_id) = typing_target(
        &input.message,
        cfg.setup_mode.as_deref(),
        cfg.default_service_url.as_deref(),
    )
    .map_err(SendTypingOutV1::failed)?;
    let token = acquire_bot_token(&cfg).map_err(SendTypingOutV1::failed)?;
    let body = serde_json::to_vec(&typing_activity_body())
        .map_err(|err| SendTypingOutV1::failed(err.to_string()))?;
    let request = client::Request {
        method: "POST".into(),
        url: connector_url(&service_url, &conversation_id),
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
            "Bot Framework typing returned status {}",
            resp.status
        )))
    }
}

/// Reads both the bot ingress keys (`service_url`) and the provider ingest keys (`serviceUrl`).
fn typing_target(
    message: &Value,
    setup_mode: Option<&str>,
    default_service_url: Option<&str>,
) -> Result<(String, String), String> {
    if setup_mode != Some("bot_framework") {
        return Err("typing needs the Bot Framework connector (bot_framework setup mode)".into());
    }
    let inbound = InboundRef::new(message);
    let service_url = inbound
        .metadata("service_url")
        .or_else(|| inbound.metadata("serviceUrl"))
        .or_else(|| default_service_url.map(str::trim).filter(|s| !s.is_empty()))
        .ok_or_else(|| "inbound envelope carries no Bot Framework service url".to_string())?;
    let conversation_id = inbound
        .metadata("conversation_id")
        .or_else(|| inbound.metadata("conversationId"))
        .or_else(|| inbound.session_id())
        .filter(|id| *id != "teams")
        .ok_or_else(|| "inbound envelope carries no Bot Framework conversation".to_string())?;
    Ok((service_url.to_string(), conversation_id.to_string()))
}

pub(crate) fn typing_activity_body() -> Value {
    json!({"type": "typing"})
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const BOT: Option<&str> = Some("bot_framework");

    #[test]
    fn the_body_is_a_bare_typing_activity() {
        assert_eq!(typing_activity_body(), json!({"type": "typing"}));
    }

    #[test]
    fn graph_mode_cannot_show_typing() {
        let msg = json!({"session_id": "a:1", "metadata": {"service_url": "https://smba/x"}});
        assert!(typing_target(&msg, None, None).is_err());
        assert!(typing_target(&msg, Some("graph"), None).is_err());
    }

    #[test]
    fn reads_the_bot_ingress_keys() {
        let msg = json!({"session_id": "a:1",
            "metadata": {"service_url": "https://smba/amer/", "conversation_id": "a:1"}});
        assert_eq!(
            typing_target(&msg, BOT, None).expect("target"),
            ("https://smba/amer/".to_string(), "a:1".to_string())
        );
    }

    #[test]
    fn reads_the_graph_ingest_camel_keys_and_config_default() {
        let camel = json!({"session_id": "teams",
            "metadata": {"serviceUrl": "https://smba/emea/", "conversationId": "c:2"}});
        assert_eq!(typing_target(&camel, BOT, None).expect("t").1, "c:2");
        let default_url = json!({"session_id": "c:3"});
        assert_eq!(
            typing_target(&default_url, BOT, Some("https://smba/default/"))
                .expect("t")
                .0,
            "https://smba/default/"
        );
    }

    #[test]
    fn the_teams_fallback_session_is_not_a_conversation() {
        let msg = json!({"session_id": "teams", "metadata": {"service_url": "https://smba/x"}});
        assert!(typing_target(&msg, BOT, None).is_err());
    }

    #[test]
    fn bad_input_and_foreign_provider_answer_ok_false() {
        let out: Value = serde_json::from_slice(&send_typing(b"{")).expect("json");
        assert_eq!(out["ok"], false);
        let foreign = json!({"v": 1, "provider_type": "messaging.slack", "message": {}});
        let out: Value =
            serde_json::from_slice(&send_typing(foreign.to_string().as_bytes())).expect("json");
        assert_eq!(out["error"], "provider type mismatch");
    }

    #[test]
    fn graph_mode_config_is_refused_before_any_call() {
        let input = json!({
            "v": 1,
            "provider_type": "messaging.teams",
            "message": {"session_id": "a:1", "metadata": {"service_url": "https://smba/x"}},
            "config": {"tenant_id": "t", "client_id": "c", "refresh_token": "r"}
        });
        let out: Value =
            serde_json::from_slice(&send_typing(input.to_string().as_bytes())).expect("json");
        assert_eq!(out["ok"], false);
        assert!(
            out["error"]
                .as_str()
                .unwrap_or("")
                .contains("bot_framework"),
            "{out}"
        );
    }
}
