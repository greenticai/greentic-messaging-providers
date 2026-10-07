use base64::{Engine as _, engine::general_purpose};
use greentic_types::messaging::universal_dto::{HttpInV1, HttpOutV1};
use greentic_types::{
    Actor, ChannelMessageEnvelope, Destination, EnvId, MessageMetadata, TenantCtx, TenantId,
};
use provider_common::attachment_fetch::apply_fetch_refs;
use provider_common::http_compat::{http_out_error, http_out_v1_bytes, parse_operator_http_in};
use provider_common::whatsapp_attachments::parse_messages;
use serde_json::{Value, json};
use std::collections::HashMap;

use crate::config::load_config;

pub(crate) fn ingest_http(input_json: &[u8]) -> Vec<u8> {
    // Try native greentic-types format first, fall back to operator format
    let request = match serde_json::from_slice::<HttpInV1>(input_json) {
        Ok(req) => req,
        Err(_) => match parse_operator_http_in(input_json) {
            Ok(req) => req,
            Err(err) => return http_out_error(400, &format!("invalid http input: {err}")),
        },
    };
    if request.method.eq_ignore_ascii_case("GET") {
        let challenge = parse_query(&request.query)
            .and_then(|params| params.get("hub.challenge").cloned())
            .unwrap_or_default();
        let out = HttpOutV1 {
            status: 200,
            headers: Vec::new(),
            body_b64: general_purpose::STANDARD.encode(challenge.as_bytes()),
            events: Vec::new(),
        };
        return http_out_v1_bytes(&out);
    }
    let body_bytes = match general_purpose::STANDARD.decode(&request.body_b64) {
        Ok(bytes) => bytes,
        Err(err) => return http_out_error(400, &format!("invalid body encoding: {err}")),
    };
    let body_val: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    // Phone number id from the Cloud API metadata (first change).
    let cloud_phone_id = body_val
        .get("entry")
        .and_then(|e| e.as_array())
        .and_then(|arr| arr.first())
        .and_then(|e| e.get("changes"))
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|c| c.get("value"))
        .and_then(|v| v.get("metadata"))
        .and_then(|m| m.get("phone_number_id"))
        .and_then(Value::as_str)
        .map(str::to_string);
    // WhatsApp doesn't include locale in webhooks; use provider config default.
    let default_locale = load_config(&body_val)
        .ok()
        .and_then(|cfg| cfg.default_locale)
        .filter(|l| !l.is_empty());
    // One envelope per message of the update, in order. Media become fetch
    // references (never a URL); the mapping is shared in provider-common.
    let mut events = Vec::new();
    let mut first_text = String::new();
    let mut first_from = None;
    for (index, msg) in parse_messages(&body_val).into_iter().enumerate() {
        if index == 0 {
            first_text = msg.text.clone();
            first_from = msg.from.clone();
        }
        let mut envelope =
            build_whatsapp_envelope(msg.text, msg.from, cloud_phone_id.clone(), msg.id);
        if let Some(locale) = &default_locale {
            envelope
                .metadata
                .insert("locale".to_string(), locale.clone());
        }
        apply_fetch_refs(&mut envelope, msg.pending);
        events.push(envelope);
    }
    let normalized = json!({
        "ok": true,
        "event": body_val,
        "text": first_text,
        "from": first_from,
    });
    let normalized_bytes = serde_json::to_vec(&normalized).unwrap_or_else(|_| b"{}".to_vec());
    let out = HttpOutV1 {
        status: 200,
        headers: Vec::new(),
        body_b64: general_purpose::STANDARD.encode(&normalized_bytes),
        events,
    };
    http_out_v1_bytes(&out)
}

fn build_whatsapp_envelope(
    text: String,
    from: Option<String>,
    phone_number_id: Option<String>,
    message_id: Option<String>,
) -> ChannelMessageEnvelope {
    let env = EnvId::try_from("default").expect("env id");
    let tenant = TenantId::try_from("default").expect("tenant id");
    let mut metadata = MessageMetadata::new();
    metadata.insert("universal".to_string(), "true".to_string());
    metadata.insert("channel_id".to_string(), "whatsapp".to_string());
    let pnid = phone_number_id.unwrap_or_else(|| "unknown".to_string());
    metadata.insert("phone_number_id".to_string(), pnid);
    // The WhatsApp message id makes envelope ids unique per message; without it
    // every media message (empty text) would collapse to `whatsapp-`. A payload
    // carrying no id keeps the old text-derived id.
    let envelope_id = match &message_id {
        Some(id) => format!("whatsapp-{id}"),
        None => format!("whatsapp-{text}"),
    };
    if let Some(id) = message_id {
        metadata.insert("wa_message_id".to_string(), id);
    }
    let sender = from.map(|id| Actor {
        id,
        kind: Some("user".into()),
    });
    if let Some(actor) = &sender {
        metadata.insert("from".to_string(), actor.id.clone());
    }
    let destinations = if let Some(actor) = &sender {
        vec![Destination {
            id: actor.id.clone(),
            kind: Some("phone".into()),
        }]
    } else {
        Vec::new()
    };
    ChannelMessageEnvelope {
        id: envelope_id,
        tenant: TenantCtx::new(env.clone(), tenant.clone()),
        channel: "whatsapp".to_string(),
        session_id: "whatsapp".to_string(),
        reply_scope: None,
        from: sender,
        to: destinations,
        correlation_id: None,
        text: Some(text),
        attachments: Vec::new(),
        metadata,
        extensions: Default::default(),
    }
}

fn parse_query(query: &Option<String>) -> Option<HashMap<String, String>> {
    let query = query.as_deref()?;
    let mut map = HashMap::new();
    for pair in query.split('&') {
        let mut parts = pair.splitn(2, '=');
        if let (Some(key), Some(value)) = (parts.next(), parts.next()) {
            map.insert(key.to_string(), value.to_string());
        }
    }
    if map.is_empty() { None } else { Some(map) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_inbound_message_id_is_kept_for_typing() {
        let msg = json!({"id": "wamid.ABC", "from": "447", "text": {"body": "hi"}});
        assert_eq!(parse_messages(&msg)[0].id.as_deref(), Some("wamid.ABC"));
        let env = build_whatsapp_envelope(
            "hi".into(),
            Some("447".into()),
            Some("pn-1".into()),
            parse_messages(&msg)[0].id.clone(),
        );
        assert_eq!(
            env.metadata.get("wa_message_id").map(String::as_str),
            Some("wamid.ABC")
        );
        let none = build_whatsapp_envelope("hi".into(), None, None, None);
        assert!(!none.metadata.contains_key("wa_message_id"));
    }

    fn http_in(body: &Value) -> Vec<u8> {
        let raw = serde_json::to_vec(body).expect("body");
        serde_json::to_vec(&json!({
            "method": "POST",
            "path": "/webhook",
            "headers": [],
            "body_b64": general_purpose::STANDARD.encode(raw),
        }))
        .expect("http in")
    }

    fn events_for(body: &Value) -> Vec<Value> {
        let out: Value = serde_json::from_slice(&ingest_http(&http_in(body))).expect("out");
        out["events"].as_array().cloned().unwrap_or_default()
    }

    fn fixture_body() -> Value {
        let v: Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/whatsapp/inbound/image_message.json"
        ))
        .expect("fixture");
        v["body"].clone()
    }

    #[test]
    fn one_envelope_per_message_with_distinct_ids() {
        let events = events_for(&fixture_body());
        assert_eq!(events.len(), 3);
        let ids: Vec<_> = events.iter().map(|e| e["id"].as_str().unwrap()).collect();
        assert_eq!(
            ids,
            ["whatsapp-wamid.A", "whatsapp-wamid.B", "whatsapp-wamid.C"]
        );
        assert_eq!(events[0]["text"], "ini apa?");
        assert_eq!(events[2]["text"], "halo");
    }

    #[test]
    fn media_messages_carry_fetch_references_never_urls() {
        let events = events_for(&fixture_body());
        let a = &events[0]["attachments"][0];
        assert_eq!(a["mime_type"], "image/jpeg");
        let dump = events[0].to_string();
        assert!(dump.contains("MEDIA1"), "{dump}");
        assert!(!dump.contains("http"), "{dump}");
        assert_eq!(events[1]["attachments"][0]["name"], "x.pdf");
        assert!(
            events[2]
                .get("attachments")
                .is_none_or(|a| a.as_array().is_some_and(Vec::is_empty))
        );
    }

    #[test]
    fn a_plain_text_message_keeps_its_shape() {
        let body = json!({"entry":[{"changes":[{"value":{
            "metadata":{"phone_number_id":"555"},
            "messages":[{"id":"wamid.T","from":"6281","text":{"body":"hi"}}]}}]}]});
        let events = events_for(&body);
        assert_eq!(events.len(), 1);
        let e = &events[0];
        assert_eq!(e["id"], "whatsapp-wamid.T");
        assert_eq!(e["text"], "hi");
        assert_eq!(e["channel"], "whatsapp");
        assert_eq!(e["metadata"]["phone_number_id"], "555");
        assert_eq!(e["metadata"]["wa_message_id"], "wamid.T");
        assert_eq!(e["from"]["id"], "6281");
        assert!(
            e.get("attachments")
                .is_none_or(|a| a.as_array().is_some_and(Vec::is_empty))
        );
    }

    #[test]
    fn a_message_without_an_id_keeps_the_text_derived_id() {
        let events = events_for(&json!({"from":"1","text":{"body":"hi"}}));
        assert_eq!(events[0]["id"], "whatsapp-hi");
    }
}
