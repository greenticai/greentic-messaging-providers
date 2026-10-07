use base64::{Engine as _, engine::general_purpose};
use greentic_types::messaging::universal_dto::{HttpInV1, HttpOutV1};
use greentic_types::{
    Actor, ChannelMessageEnvelope, Destination, EnvId, MessageMetadata, TenantCtx, TenantId,
};
use provider_common::attachment_fetch::apply_fetch_refs;
use provider_common::http_compat::{http_out_error, http_out_v1_bytes, parse_operator_http_in};
use provider_common::whatsapp_attachments::{
    is_readable_wamid, parse_update, whatsapp_envelope_id,
};
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
    let update = parse_update(&body_val);
    let last = update.messages.len().saturating_sub(1);
    for (index, msg) in update.messages.into_iter().enumerate() {
        if index == 0 {
            first_text = msg.text.clone();
            first_from = msg.from.clone();
        }
        let mut envelope = build_whatsapp_envelope(msg.text, msg.from, msg.phone_number_id, msg.id);
        if let Some(locale) = &default_locale {
            envelope
                .metadata
                .insert("locale".to_string(), locale.clone());
        }
        apply_fetch_refs(&mut envelope, msg.pending);
        if index == last && update.dropped > 0 {
            envelope
                .metadata
                .insert("messages_dropped".to_string(), update.dropped.to_string());
        }
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
        Some(id) => whatsapp_envelope_id(id),
        None => format!("whatsapp-{text}"),
    };
    // Typing needs the raw id; an unreadable one is not carried at all.
    if let Some(id) = message_id.filter(|id| is_readable_wamid(id)) {
        metadata.insert("wa_message_id".to_string(), id.trim().to_string());
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
        assert_eq!(
            provider_common::whatsapp_attachments::parse_messages(&msg)[0]
                .id
                .as_deref(),
            Some("wamid.ABC")
        );
        let env = build_whatsapp_envelope(
            "hi".into(),
            Some("447".into()),
            Some("pn-1".into()),
            provider_common::whatsapp_attachments::parse_messages(&msg)[0]
                .id
                .clone(),
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
    fn each_envelope_carries_the_phone_number_of_its_own_change() {
        let body = json!({"entry":[
            {"changes":[
                {"value":{"metadata":{"phone_number_id":"111"},
                    "messages":[{"id":"wamid.1","from":"6281","text":{"body":"a"}}]}},
                {"value":{"metadata":{"phone_number_id":"222"},
                    "messages":[{"id":"wamid.2","from":"6282","text":{"body":"b"}}]}}]},
            {"changes":[
                {"value":{"metadata":{"phone_number_id":"333"},
                    "messages":[{"id":"wamid.3","from":"6283","text":{"body":"c"}}]}},
                {"value":{"messages":[{"id":"wamid.4","from":"6284","text":{"body":"d"}}]}}]}]});
        let numbers: Vec<Value> = events_for(&body)
            .iter()
            .map(|e| e["metadata"]["phone_number_id"].clone())
            .collect();
        assert_eq!(
            numbers,
            [json!("111"), json!("222"), json!("333"), json!("unknown")]
        );
    }

    #[test]
    fn an_unreadable_wamid_is_not_stamped_as_wa_message_id() {
        for wamid in ["wamid. with space", "../x", &"w".repeat(129)] {
            let env = build_whatsapp_envelope("hi".into(), None, None, Some(wamid.to_string()));
            assert!(env.id.starts_with("whatsapp-~"), "{wamid}");
            assert!(!env.metadata.contains_key("wa_message_id"), "{wamid}");
        }
        let ok = build_whatsapp_envelope("hi".into(), None, None, Some("wamid.HBgM=".into()));
        assert_eq!(
            ok.metadata.get("wa_message_id").map(String::as_str),
            Some("wamid.HBgM=")
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

    fn out_for(body: &Value) -> Value {
        serde_json::from_slice(&ingest_http(&http_in(body))).expect("out")
    }

    fn one_message(m: Value) -> Value {
        json!({"entry":[{"changes":[{"value":{"messages":[m]}}]}]})
    }

    #[test]
    fn a_delivery_receipt_creates_no_envelope() {
        let body = json!({"entry":[{"changes":[{"value":{
            "metadata":{"phone_number_id":"555"},
            "statuses":[{"id":"wamid.S","status":"delivered","recipient_id":"1"}]}}]}]});
        let out = out_for(&body);
        assert_eq!(out["status"], 200);
        assert_eq!(out["events"].as_array().map(Vec::len), Some(0));
    }

    #[test]
    fn a_flat_payload_without_entry_still_makes_one_envelope() {
        let events = events_for(&json!({"from":"1","text":{"body":"hi"}}));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["text"], "hi");
    }

    #[test]
    fn messages_beyond_the_cap_are_dropped_and_counted() {
        let many: Vec<Value> = (0..250)
            .map(|i| json!({"id": format!("w{i}"), "from": "1", "text": {"body": "x"}}))
            .collect();
        let events = events_for(&json!({"entry":[{"changes":[{"value":{"messages": many}}]}]}));
        assert_eq!(events.len(), 100);
        assert_eq!(events[0]["metadata"].get("messages_dropped"), None);
        assert_eq!(events[99]["metadata"]["messages_dropped"], "150");
    }

    #[test]
    fn a_hostile_message_id_cannot_shape_the_envelope_id() {
        let long = "x".repeat(10 * 1024);
        for hostile in ["a\nb", "a/b", long.as_str()] {
            let events = events_for(&one_message(json!({"id": hostile, "text": {"body": "t"}})));
            let id = events[0]["id"].as_str().unwrap();
            assert!(id.starts_with("whatsapp-~") && id.len() < 64, "{id}");
        }
    }

    #[test]
    fn hostile_media_ids_are_refused_but_the_turn_survives() {
        let long = "9".repeat(10 * 1024);
        for bad in [
            json!("../x"),
            json!("https://evil/x"),
            json!(long),
            json!(12345),
        ] {
            let events = events_for(&one_message(json!({
                "id": "wamid.H", "from": "1", "type": "image",
                "image": {"id": bad, "mime_type": "image/png", "caption": "keep me"}})));
            assert_eq!(events.len(), 1);
            assert_eq!(events[0]["text"], "keep me");
            assert!(
                events[0]
                    .get("attachments")
                    .is_none_or(|a| a.as_array().is_some_and(Vec::is_empty))
            );
            assert!(events[0]["extensions"].get("attachment_fetch").is_none());
        }
        // A well-formed id with a hostile neighbour is the case the shared
        // refusal covers: it must be dropped and counted, not fail the turn.
        let events = events_for(&one_message(json!({
            "id": "wamid.H", "type": "document",
            "document": {"id": "../x", "mime_type": "application/pdf", "caption": "c"}})));
        assert_eq!(events[0]["text"], "c");
        assert_eq!(events[0]["metadata"]["attachments_dropped"], "1");
    }

    #[test]
    fn reaction_button_and_interactive_messages_stay_empty() {
        for m in [
            json!({"id":"r","from":"1","type":"reaction","reaction":{"message_id":"x","emoji":"y"}}),
            json!({"id":"b","from":"1","type":"button","button":{"text":"Yes"}}),
            json!({"id":"i","from":"1","type":"interactive","interactive":{"type":"button_reply"}}),
        ] {
            let events = events_for(&one_message(m));
            assert_eq!(events.len(), 1);
            assert_eq!(events[0]["text"], "");
            assert!(
                events[0]
                    .get("attachments")
                    .is_none_or(|a| a.as_array().is_some_and(Vec::is_empty))
            );
        }
    }

    #[test]
    fn text_and_an_image_in_one_message_keep_the_text() {
        let events = events_for(&one_message(json!({
            "id":"wamid.X","from":"1","type":"image","text":{"body":"the text"},
            "image":{"id":"M9","mime_type":"image/png","caption":"the caption"}})));
        assert_eq!(events[0]["text"], "the text");
        assert_eq!(
            events[0]["extensions"]["attachment_fetch"][0]["media_id"],
            "M9"
        );
    }
}
