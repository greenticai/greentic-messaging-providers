//! `send_typing`: raise the "bot is typing" indicator in a Direct Line
//! conversation. Writes ONLY the ephemeral `<conv_key>:typing` slot that
//! `GET …/activities` reads; the conversation document is never touched.

use provider_common::typing::{InboundRef, SendTypingOutV1, parse_send_typing};
use serde_json::{Value, json};

use crate::PROVIDER_TYPE;
use crate::directline::HostStateStore;
use crate::directline::jwt::DirectLineContext;
use crate::directline::state::{ConversationState, TypingSlot, typing_key};
use crate::directline::store::StateStore;

use super::send_payload::find_existing_conversation_state;

pub(crate) const WEBCHAT_TYPING_REFRESH_MS: u64 = 4_000;

pub(crate) fn send_typing(input_json: &[u8]) -> Vec<u8> {
    let input = match parse_send_typing(input_json, &[PROVIDER_TYPE]) {
        Ok(input) => input,
        Err(out) => return out.to_bytes(),
    };
    let (conversation_id, ctx) = match typing_target(&input.message) {
        Ok(target) => target,
        Err(err) => return SendTypingOutV1::failed(err).to_bytes(),
    };
    let mut store = HostStateStore;
    let now_ms = chrono::Utc::now().timestamp_millis();
    match raise_typing(&mut store, &conversation_id, &ctx, now_ms) {
        Ok(meta) => SendTypingOutV1::raised(WEBCHAT_TYPING_REFRESH_MS)
            .with_greentic(meta)
            .to_bytes(),
        Err(err) => SendTypingOutV1::failed(err).to_bytes(),
    }
}

/// `"webchat"` is the ingest fallback for "no conversation" and is refused.
fn typing_target(message: &Value) -> Result<(String, DirectLineContext), String> {
    let inbound = InboundRef::new(message);
    let conversation_id = inbound
        .session_id()
        .filter(|id| *id != "webchat")
        .ok_or_else(|| "inbound envelope carries no Direct Line conversation".to_string())?;
    let env = inbound
        .metadata("env")
        .or_else(|| inbound.tenant_field("env"))
        .unwrap_or("default");
    let tenant = inbound
        .metadata("tenant")
        .or_else(|| inbound.tenant_field("tenant"))
        .unwrap_or("default");
    let team = inbound
        .metadata("team")
        .or_else(|| inbound.tenant_field("team"))
        .map(str::to_string);
    Ok((
        conversation_id.to_string(),
        DirectLineContext {
            env: env.to_string(),
            tenant: tenant.to_string(),
            team,
        },
    ))
}

fn raise_typing<S: StateStore>(
    store: &mut S,
    conversation_id: &str,
    ctx: &DirectLineContext,
    now_ms: i64,
) -> Result<Value, String> {
    let (conv_key, bytes) = find_existing_conversation_state(store, ctx, conversation_id)?
        .ok_or_else(|| "conversation not found".to_string())?;
    let conversation: ConversationState =
        serde_json::from_slice(&bytes).map_err(|err| format!("conversation state: {err}"))?;
    let slot = TypingSlot::raise(&conversation, now_ms);
    let slot_bytes = serde_json::to_vec(&slot).map_err(|err| err.to_string())?;
    store.write(&typing_key(&conv_key), &slot_bytes)?;
    Ok(json!({
        "watermark_bumped": conversation.next_watermark,
        "conversation_id": conversation_id,
        "tenant": conversation.ctx.tenant,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::directline::state::conversation_key;
    use serde_json::json;
    use std::collections::HashMap;

    #[derive(Default)]
    struct MemStore(HashMap<String, Vec<u8>>);
    impl StateStore for MemStore {
        fn read(&mut self, key: &str) -> Result<Option<Vec<u8>>, String> {
            Ok(self.0.get(key).cloned())
        }
        fn write(&mut self, key: &str, value: &[u8]) -> Result<(), String> {
            self.0.insert(key.to_string(), value.to_vec());
            Ok(())
        }
    }

    fn ctx() -> DirectLineContext {
        DirectLineContext {
            env: "prod".into(),
            tenant: "acme".into(),
            team: None,
        }
    }

    fn seeded_store(next_watermark: u64) -> (MemStore, String) {
        let mut store = MemStore::default();
        let mut conv = ConversationState::new(ctx());
        conv.next_watermark = next_watermark;
        let key = conversation_key(&ctx(), "conv-1");
        store
            .0
            .insert(key.clone(), serde_json::to_vec(&conv).expect("json"));
        (store, key)
    }

    #[test]
    fn target_comes_from_session_and_metadata_context() {
        let msg = json!({"session_id": "conv-1", "metadata": {"env": "prod", "tenant": "acme"}});
        let (id, target_ctx) = typing_target(&msg).expect("target");
        assert_eq!(id, "conv-1");
        assert_eq!(target_ctx, ctx());
    }

    #[test]
    fn target_falls_back_to_the_envelope_tenant_ctx() {
        let msg = json!({"session_id": "conv-1",
            "tenant": {"env": "prod", "tenant": "acme", "tenant_id": "acme"}});
        assert_eq!(typing_target(&msg).expect("target").1, ctx());
    }

    #[test]
    fn the_no_conversation_fallback_is_refused() {
        for msg in [
            json!({"session_id": "webchat"}),
            json!({}),
            json!({"session_id": " "}),
        ] {
            assert!(typing_target(&msg).is_err(), "{msg}");
        }
    }

    #[test]
    fn raise_writes_only_the_typing_key_and_leaves_the_conversation_untouched() {
        let (mut store, conv_key) = seeded_store(4);
        let before = store.0.get(&conv_key).cloned();
        let meta = raise_typing(&mut store, "conv-1", &ctx(), 10_000).expect("raised");
        assert_eq!(
            store.0.get(&conv_key).cloned(),
            before,
            "conversation not rewritten"
        );
        assert_eq!(store.0.len(), 2);
        let slot: TypingSlot =
            serde_json::from_slice(store.0.get(&typing_key(&conv_key)).expect("slot written"))
                .expect("slot json");
        assert_eq!(slot.since_watermark, 4);
        assert_eq!(
            meta,
            json!({"watermark_bumped": 4, "conversation_id": "conv-1", "tenant": "acme"})
        );
    }

    #[test]
    fn raise_on_an_unknown_conversation_is_an_error_not_a_new_conversation() {
        let mut store = MemStore::default();
        assert!(raise_typing(&mut store, "missing", &ctx(), 0).is_err());
        assert!(store.0.is_empty());
    }

    #[test]
    fn bad_input_answers_ok_false() {
        let out: Value = serde_json::from_slice(&send_typing(b"{")).expect("json");
        assert_eq!(out["ok"], false);
        let foreign = json!({"v": 1, "provider_type": "messaging.slack", "message": {}});
        let out: Value =
            serde_json::from_slice(&send_typing(foreign.to_string().as_bytes())).expect("json");
        assert_eq!(out["error"], "provider type mismatch");
    }
}
