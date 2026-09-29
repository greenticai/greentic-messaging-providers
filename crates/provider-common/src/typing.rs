//! `send_typing`: the optional "the bot is typing" provider op (typing signal v1).
//!
//! A provider opts in by listing `send_typing` in its pack's
//! `greentic.provider-extension.v1` ops. Typing is cosmetic: every failure is
//! `ok: false`, never a trap, and no implementation may produce a visible
//! message or stored history.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::helpers::json_bytes;

pub const SEND_TYPING_OP: &str = "send_typing";
pub const SEND_TYPING_VERSION: u32 = 1;

fn default_version() -> u32 {
    SEND_TYPING_VERSION
}

/// Unknown keys (e.g. the host's `tenant` hint) are ignored on purpose.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SendTypingInV1 {
    #[serde(default = "default_version")]
    pub v: u32,
    pub provider_type: String,
    #[serde(default)]
    pub tenant_id: Option<String>,
    /// The inbound envelope the turn answers, kept loose so a newer envelope never fails parsing.
    pub message: Value,
    #[serde(default)]
    pub config: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SendTypingOutV1 {
    pub v: u32,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_after_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(rename = "_greentic", default, skip_serializing_if = "Option::is_none")]
    pub greentic: Option<Value>,
}

impl SendTypingOutV1 {
    pub fn raised(refresh_after_ms: u64) -> Self {
        Self {
            v: SEND_TYPING_VERSION,
            ok: true,
            refresh_after_ms: Some(refresh_after_ms),
            error: None,
            greentic: None,
        }
    }

    pub fn failed(error: impl Into<String>) -> Self {
        Self {
            v: SEND_TYPING_VERSION,
            ok: false,
            refresh_after_ms: None,
            error: Some(error.into()),
            greentic: None,
        }
    }

    pub fn with_greentic(mut self, meta: Value) -> Self {
        self.greentic = Some(meta);
        self
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        json_bytes(self)
    }
}

pub fn parse_send_typing(
    input_json: &[u8],
    accepted_provider_types: &[&str],
) -> Result<SendTypingInV1, SendTypingOutV1> {
    let input: SendTypingInV1 = serde_json::from_slice(input_json)
        .map_err(|err| SendTypingOutV1::failed(format!("invalid send_typing input: {err}")))?;
    if input.v != SEND_TYPING_VERSION {
        return Err(SendTypingOutV1::failed(format!(
            "unsupported send_typing version {}",
            input.v
        )));
    }
    if !accepted_provider_types.contains(&input.provider_type.as_str()) {
        return Err(SendTypingOutV1::failed("provider type mismatch"));
    }
    if !input.message.is_object() {
        return Err(SendTypingOutV1::failed(
            "message must be the inbound envelope object",
        ));
    }
    Ok(input)
}

/// `{"config": cfg}` when the host sent config, else `{}`.
pub fn config_input(input: &SendTypingInV1) -> Value {
    match &input.config {
        Some(cfg) if !cfg.is_null() => json!({ "config": cfg }),
        _ => json!({}),
    }
}

/// Read-only accessors over the inbound envelope; each yields a trimmed, non-empty string.
pub struct InboundRef<'a>(&'a Value);

impl<'a> InboundRef<'a> {
    pub fn new(message: &'a Value) -> Self {
        Self(message)
    }

    fn text(value: Option<&'a Value>) -> Option<&'a str> {
        value
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }

    pub fn session_id(&self) -> Option<&'a str> {
        Self::text(self.0.get("session_id"))
    }

    pub fn metadata(&self, key: &str) -> Option<&'a str> {
        Self::text(self.0.get("metadata").and_then(|m| m.get(key)))
    }

    pub fn tenant_field(&self, key: &str) -> Option<&'a str> {
        Self::text(self.0.get("tenant").and_then(|t| t.get(key)))
    }

    pub fn first_destination(&self) -> Option<&'a str> {
        Self::text(
            self.0
                .get("to")
                .and_then(Value::as_array)
                .and_then(|to| to.first())
                .and_then(|d| d.get("id")),
        )
    }

    pub fn from_id(&self) -> Option<&'a str> {
        Self::text(self.0.get("from").and_then(|f| f.get("id")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const WEBCHAT: &[&str] = &["messaging.webchat"];

    fn input(extra: Value) -> Vec<u8> {
        let mut base = json!({
            "v": 1,
            "provider_type": "messaging.webchat",
            "message": {"session_id": "conv-1", "metadata": {"env": "prod"}}
        });
        if let (Some(b), Some(e)) = (base.as_object_mut(), extra.as_object()) {
            for (k, v) in e {
                b.insert(k.clone(), v.clone());
            }
        }
        serde_json::to_vec(&base).expect("json")
    }

    #[test]
    fn parses_host_input_with_tenant_hint_and_string_tenant_id() {
        let parsed = parse_send_typing(
            &input(json!({"tenant": {"env": "prod", "tenant": "acme"}, "tenant_id": "acme"})),
            WEBCHAT,
        )
        .expect("parses");
        assert_eq!(parsed.v, 1);
        assert_eq!(parsed.provider_type, "messaging.webchat");
        assert_eq!(parsed.tenant_id.as_deref(), Some("acme"));
        assert!(parsed.config.is_none());
    }

    #[test]
    fn v_defaults_to_one_when_absent() {
        let raw = br#"{"provider_type":"messaging.webchat","message":{}}"#;
        assert_eq!(parse_send_typing(raw, WEBCHAT).expect("parses").v, 1);
    }

    #[test]
    fn refuses_bad_json_wrong_provider_version_and_non_object_message() {
        for (raw, needle) in [
            (b"{".to_vec(), "invalid send_typing input"),
            (
                input(json!({"provider_type": "messaging.slack"})),
                "provider type mismatch",
            ),
            (input(json!({"v": 2})), "unsupported send_typing version"),
            (input(json!({"message": "hi"})), "inbound envelope"),
        ] {
            let out = parse_send_typing(&raw, WEBCHAT).expect_err("refused");
            assert!(!out.ok);
            assert!(out.refresh_after_ms.is_none());
            let err = out.error.unwrap_or_default();
            assert!(err.contains(needle), "{err} should contain {needle}");
        }
    }

    #[test]
    fn raised_output_serializes_the_contract_shape() {
        let value: Value =
            serde_json::from_slice(&SendTypingOutV1::raised(4000).to_bytes()).expect("json");
        assert_eq!(value, json!({"v": 1, "ok": true, "refresh_after_ms": 4000}));
    }

    #[test]
    fn failed_output_has_no_refresh_and_greentic_block_is_underscored() {
        let failed: Value =
            serde_json::from_slice(&SendTypingOutV1::failed("nope").to_bytes()).expect("json");
        assert_eq!(failed, json!({"v": 1, "ok": false, "error": "nope"}));

        let with_meta: Value = serde_json::from_slice(
            &SendTypingOutV1::raised(1)
                .with_greentic(json!({"watermark_bumped": 3}))
                .to_bytes(),
        )
        .expect("json");
        assert_eq!(with_meta["_greentic"]["watermark_bumped"], 3);
    }

    #[test]
    fn config_input_wraps_config_or_is_empty() {
        let parsed =
            parse_send_typing(&input(json!({"config": {"a": 1}})), WEBCHAT).expect("parses");
        assert_eq!(config_input(&parsed), json!({"config": {"a": 1}}));
        let bare = parse_send_typing(&input(json!({"config": null})), WEBCHAT).expect("p");
        assert_eq!(config_input(&bare), json!({}));
    }

    #[test]
    fn inbound_ref_reads_trimmed_non_empty_strings_only() {
        let msg = json!({
            "session_id": " conv-1 ",
            "tenant": {"env": "prod", "tenant": "acme", "team": ""},
            "to": [{"id": "777"}],
            "from": {"id": "alice"},
            "metadata": {"chat_id": "777", "blank": "  ", "num": 5}
        });
        let r = InboundRef::new(&msg);
        assert_eq!(r.session_id(), Some("conv-1"));
        assert_eq!(r.tenant_field("env"), Some("prod"));
        assert_eq!(r.tenant_field("team"), None);
        assert_eq!(r.first_destination(), Some("777"));
        assert_eq!(r.from_id(), Some("alice"));
        assert_eq!(r.metadata("chat_id"), Some("777"));
        assert_eq!(r.metadata("blank"), None);
        assert_eq!(r.metadata("num"), None);
        assert_eq!(InboundRef::new(&json!({})).session_id(), None);
    }
}
