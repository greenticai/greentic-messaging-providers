mod bindings {
    wit_bindgen::generate!({
        path: "wit/messaging-ingress-slack",
        world: "messaging-ingress-slack",
        generate_all
    });
}

use bindings::exports::provider::common0_0_2::ingress::Guest;
use bindings::exports::provider::common0_0_3::ingress::Guest as ConfiguredIngressGuest;
use bindings::greentic::secrets_store::secrets_store;
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{Map, Value, json};
use sha2::Sha256;

const SIGNING_SECRET_KEY: &str = "SLACK_SIGNING_SECRET";
const USER_ENTERED_EVENT_TYPE: &str = "channel.user.entered";

struct Component;

impl Guest for Component {
    fn handle_webhook(headers_json: String, body_json: String) -> Result<String, String> {
        let headers: Map<String, Value> = serde_json::from_str(&headers_json)
            .map_err(|_| "validation error: invalid headers".to_string())?;

        if let Some(secret_result) = get_optional_secret(SIGNING_SECRET_KEY) {
            let signing_secret = secret_result.map_err(|e| format!("transport error: {e}"))?;
            verify_signature(&headers, &body_json, &signing_secret)?;
        }

        normalize_body(&headers, &body_json)
    }
}

impl ConfiguredIngressGuest for Component {
    fn handle_webhook(
        headers_json: String,
        body_json: String,
        _config_json: String,
    ) -> Result<String, String> {
        <Component as Guest>::handle_webhook(headers_json, body_json)
    }
}

bindings::exports::provider::common0_0_2::ingress::__export_provider_common_ingress_0_0_2_cabi!(
    Component with_types_in bindings::exports::provider::common0_0_2::ingress
);
bindings::exports::provider::common0_0_3::ingress::__export_provider_common_ingress_0_0_3_cabi!(
    Component with_types_in bindings::exports::provider::common0_0_3::ingress
);

fn get_optional_secret(key: &str) -> Option<Result<String, String>> {
    match secrets_store::get(key) {
        Ok(Some(bytes)) => {
            Some(String::from_utf8(bytes).map_err(|_| "secret not valid utf-8".into()))
        }
        Ok(None) => None,
        Err(e) => Some(Err(format!("secret store error: {e:?}"))),
    }
}

fn verify_signature(headers: &Map<String, Value>, body: &str, secret: &str) -> Result<(), String> {
    let signature = header_value(headers, "x-slack-signature")
        .ok_or_else(|| "validation error: missing signature".to_string())?;
    let timestamp = header_value(headers, "x-slack-request-timestamp")
        .ok_or_else(|| "validation error: missing timestamp".to_string())?;

    let basestring = format!("v0:{timestamp}:{body}");
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .map_err(|_| "validation error: invalid secret".to_string())?;
    mac.update(basestring.as_bytes());
    let signature_bytes = mac.finalize().into_bytes();
    let computed = format!("v0={}", hex_encode(&signature_bytes));

    if computed == signature {
        Ok(())
    } else {
        Err("validation error: invalid signature".to_string())
    }
}

fn normalize_body(headers: &Map<String, Value>, body_json: &str) -> Result<String, String> {
    let body_val = parse_slack_body(body_json)?;
    if body_val.get("type").and_then(Value::as_str) == Some("url_verification") {
        let challenge = body_val
            .get("challenge")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let normalized = json!({
            "status": 200,
            "headers": {"content-type": "text/plain"},
            "body": challenge,
            "events": [],
            "ok": true,
            "event": body_val,
        });
        return serde_json::to_string(&normalized)
            .map_err(|_| "other error: serialization failed".to_string());
    }

    if is_slack_retry(headers) {
        return normalized_without_events(body_val);
    }

    let payload = body_val
        .get("event")
        .or_else(|| body_val.get("body"))
        .cloned()
        .unwrap_or_else(|| body_val.clone());

    if is_bot_message(&payload) {
        return normalized_without_events(body_val);
    }

    let mut events = Vec::new();
    if let Some(envelope) = lifecycle_envelope_from_payload(&body_val, &payload) {
        events.push(envelope);
    } else if let Some(envelope) = envelope_from_payload(&payload) {
        events.push(envelope);
    }

    let normalized = json!({
        "ok": true,
        "event": body_val,
        "events": events,
    });
    serde_json::to_string(&normalized).map_err(|_| "other error: serialization failed".to_string())
}

fn lifecycle_envelope_from_payload(body: &Value, payload: &Value) -> Option<Value> {
    let event_type = payload.get("type").and_then(Value::as_str)?;
    let team_id = slack_team_id(body);
    let (reason, session_id, channel_id, user_id) = match event_type {
        "app_home_opened" => {
            let user = payload.get("user").and_then(Value::as_str)?;
            let channel = payload.get("channel").and_then(Value::as_str);
            (
                "app_home_opened",
                channel.unwrap_or(user).to_string(),
                channel,
                Some(user),
            )
        }
        "member_joined_channel" => {
            let channel = payload.get("channel").and_then(Value::as_str)?;
            let user = payload.get("user").and_then(Value::as_str)?;
            (
                "member_joined_channel",
                channel.to_string(),
                Some(channel),
                Some(user),
            )
        }
        _ => return None,
    };
    let mut envelope = envelope_from_parts(
        "",
        Some(session_id.as_str()),
        user_id,
        payload
            .get("event_ts")
            .or_else(|| payload.get("ts"))
            .and_then(Value::as_str),
    );
    let metadata = envelope
        .get_mut("metadata")
        .and_then(Value::as_object_mut)
        .expect("envelope metadata");
    metadata.insert(
        "event_type".to_string(),
        Value::String(USER_ENTERED_EVENT_TYPE.to_string()),
    );
    metadata.insert("autoStart".to_string(), Value::String("true".to_string()));
    metadata.insert("provider".to_string(), Value::String("slack".to_string()));
    metadata.insert("reason".to_string(), Value::String(reason.to_string()));
    metadata.insert(
        "idempotency_key".to_string(),
        Value::String(user_entered_idempotency_key(
            "slack",
            team_id.as_deref(),
            channel_id.or(Some(session_id.as_str())),
            user_id,
            reason,
        )),
    );
    if let Some(team_id) = team_id {
        metadata.insert("team_id".to_string(), Value::String(team_id));
    }
    if let Some(channel_id) = channel_id {
        metadata.insert(
            "channel_id".to_string(),
            Value::String(channel_id.to_string()),
        );
    } else {
        metadata.remove("channel");
    }
    if let Some(user_id) = user_id {
        metadata.insert("user_id".to_string(), Value::String(user_id.to_string()));
    }
    Some(envelope)
}

fn parse_slack_body(body_json: &str) -> Result<Value, String> {
    if let Ok(value) = serde_json::from_str(body_json) {
        return Ok(value);
    }
    if let Some(payload) = body_json.strip_prefix("payload=") {
        return serde_json::from_str(&url_decode(payload))
            .map_err(|_| "validation error: invalid body json".to_string());
    }
    Err("validation error: invalid body json".to_string())
}

fn normalized_without_events(body_val: Value) -> Result<String, String> {
    let normalized = json!({
        "ok": true,
        "event": body_val,
        "events": [],
    });
    serde_json::to_string(&normalized).map_err(|_| "other error: serialization failed".to_string())
}

fn envelope_from_payload(payload: &Value) -> Option<Value> {
    let (text, mut action_metadata) =
        if payload.get("type").and_then(Value::as_str) == Some("block_actions") {
            block_action_text_and_metadata(payload)
        } else {
            (
                payload
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                Map::new(),
            )
        };
    let channel = slack_channel_id(payload);
    let sender = slack_user_id(payload);
    let mut envelope = envelope_from_parts(
        &text,
        channel.as_deref(),
        sender.as_deref(),
        payload
            .get("event_ts")
            .or_else(|| payload.get("ts"))
            .and_then(Value::as_str),
    );
    let metadata = envelope
        .get_mut("metadata")
        .and_then(Value::as_object_mut)
        .expect("envelope metadata");
    if let Some(ts) = payload.get("ts").and_then(Value::as_str) {
        metadata.insert("ts".to_string(), Value::String(ts.to_string()));
    }
    if let Some(event_ts) = payload.get("event_ts").and_then(Value::as_str) {
        metadata.insert("event_ts".to_string(), Value::String(event_ts.to_string()));
    }
    metadata.append(&mut action_metadata);

    Some(envelope)
}

fn envelope_from_parts(
    text: &str,
    channel: Option<&str>,
    sender: Option<&str>,
    event_ts: Option<&str>,
) -> Value {
    let channel = channel.map(ToOwned::to_owned);
    let sender = sender.map(ToOwned::to_owned);
    let channel_name = channel.clone().unwrap_or_else(|| "slack".to_string());
    let envelope_id = event_ts
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!("slack:{value}"))
        .unwrap_or_else(|| format!("slack-{channel_name}"));

    let mut metadata = Map::new();
    metadata.insert("universal".to_string(), Value::String("true".to_string()));
    if let Some(channel_id) = channel.as_ref() {
        metadata.insert("channel".to_string(), Value::String(channel_id.clone()));
    }
    if let Some(sender_id) = sender.as_ref() {
        metadata.insert("from".to_string(), Value::String(sender_id.clone()));
    }

    json!({
        "id": envelope_id,
        "tenant": {
            "env": "default",
            "tenant": "default",
            "tenant_id": "default",
            "attempt": 0
        },
        "channel": channel_name,
        "session_id": channel.clone().unwrap_or_else(|| "slack".to_string()),
        "from": sender.map(|id| json!({"id": id, "kind": "user"})),
        "to": channel.map(|id| vec![json!({"id": id})]).unwrap_or_default(),
        "text": text,
        "attachments": [],
        "metadata": metadata,
    })
}

fn block_action_text_and_metadata(payload: &Value) -> (String, Map<String, Value>) {
    let first_action = payload
        .get("actions")
        .and_then(Value::as_array)
        .and_then(|actions| actions.first())
        .cloned()
        .unwrap_or(Value::Null);
    let action_id = first_action
        .get("action_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let action_value = first_action
        .get("value")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let parsed_value = serde_json::from_str::<Value>(action_value).ok();
    let text = if let Some(value) = parsed_value.as_ref() {
        let route_to_card = value
            .get("routeToCardId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let card_id = value
            .get("cardId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !route_to_card.is_empty() {
            format!("[card:{route_to_card}]")
        } else if !card_id.is_empty() {
            format!("[action:{card_id}]")
        } else {
            format!("[action:{action_id}]")
        }
    } else {
        format!("[action:{action_id}]")
    };

    let mut metadata = Map::new();
    if let Some(obj) = parsed_value.as_ref().and_then(Value::as_object) {
        for (key, value) in obj {
            let value = match value {
                Value::String(value) => Value::String(value.clone()),
                other => Value::String(other.to_string()),
            };
            metadata.insert(key.clone(), value);
        }
    }
    metadata.insert(
        "slack.action_id".to_string(),
        Value::String(action_id.to_string()),
    );
    metadata.insert(
        "slack.action_value".to_string(),
        Value::String(action_value.to_string()),
    );
    (text, metadata)
}

fn slack_channel_id(payload: &Value) -> Option<String> {
    payload
        .get("channel")
        .and_then(|channel| {
            channel
                .as_str()
                .or_else(|| channel.get("id").and_then(Value::as_str))
        })
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
}

fn slack_user_id(payload: &Value) -> Option<String> {
    payload
        .get("user")
        .or_else(|| payload.get("user_id"))
        .and_then(|user| {
            user.as_str()
                .or_else(|| user.get("id").and_then(Value::as_str))
        })
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
}

fn slack_team_id(body: &Value) -> Option<String> {
    body.get("team_id")
        .or_else(|| body.get("team"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            body.get("authorizations")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(|item| item.get("team_id"))
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(ToOwned::to_owned)
        })
}

fn user_entered_idempotency_key(
    provider: &str,
    scope: Option<&str>,
    conversation: Option<&str>,
    user: Option<&str>,
    reason: &str,
) -> String {
    format!(
        "lifecycle.user_entered:{}:{}:{}:{}:{}",
        key_part(provider),
        key_part(scope.unwrap_or_default()),
        key_part(conversation.unwrap_or_default()),
        key_part(user.unwrap_or_default()),
        key_part(reason)
    )
}

fn key_part(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "_".to_string()
    } else {
        trimmed.replace(':', "_")
    }
}

fn is_slack_retry(headers: &Map<String, Value>) -> bool {
    header_value(headers, "x-slack-retry-num").is_some()
}

fn is_bot_message(payload: &Value) -> bool {
    payload.get("bot_id").is_some_and(|value| !value.is_null())
        || payload.get("subtype").and_then(Value::as_str) == Some("bot_message")
}

fn url_decode(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(ch) = chars.next() {
        if ch == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                result.push(byte as char);
            } else {
                result.push('%');
                result.push_str(&hex);
            }
        } else if ch == '+' {
            result.push(' ');
        } else {
            result.push(ch);
        }
    }
    result
}

fn header_value(headers: &Map<String, Value>, key: &str) -> Option<String> {
    headers
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .and_then(|(_, v)| v.as_str())
                .map(|s| s.to_string())
        })
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{:02x}", b));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_lookup_is_case_insensitive() {
        let mut headers = Map::new();
        headers.insert("X-Slack-Signature".to_string(), Value::String("sig".into()));

        assert_eq!(
            header_value(&headers, "x-slack-signature").as_deref(),
            Some("sig")
        );
    }

    #[test]
    fn signature_verification_accepts_expected_hmac() {
        let body = r#"{"type":"event_callback"}"#;
        let timestamp = "1700000000";
        let signing_key = ["signing", "key"].join("-");
        let basestring = format!("v0:{timestamp}:{body}");
        let mut mac = Hmac::<Sha256>::new_from_slice(signing_key.as_bytes()).expect("hmac");
        mac.update(basestring.as_bytes());
        let signature = format!("v0={}", hex_encode(&mac.finalize().into_bytes()));
        let mut headers = Map::new();
        headers.insert(
            "x-slack-signature".to_string(),
            Value::String(signature.clone()),
        );
        headers.insert(
            "x-slack-request-timestamp".to_string(),
            Value::String(timestamp.into()),
        );

        verify_signature(&headers, body, &signing_key).expect("valid signature");

        headers.insert(
            "x-slack-signature".to_string(),
            Value::String(format!("{signature}bad")),
        );
        let err = verify_signature(&headers, body, &signing_key).expect_err("bad signature");
        assert!(err.contains("invalid signature"), "{err}");
    }

    #[test]
    fn webhook_normalizes_valid_json_without_signing_secret() {
        let body = r#"{"type":"event_callback","event":{"text":"hi","channel":"C1","user":"U1","event_ts":"1780414157.651"}}"#;
        let headers = Map::new();

        let out = normalize_body(&headers, body).expect("normalized");
        let parsed: Value = serde_json::from_str(&out).expect("json");

        assert_eq!(parsed["ok"], true);
        assert_eq!(parsed["event"]["event"]["text"], "hi");
        assert_eq!(parsed["events"].as_array().expect("events").len(), 1);
        let envelope: greentic_types::ChannelMessageEnvelope =
            serde_json::from_value(parsed["events"][0].clone()).expect("channel envelope");
        assert_eq!(envelope.id, "slack:1780414157.651");
        assert_eq!(envelope.channel, "C1");
        assert_eq!(envelope.session_id, "C1");
        assert_eq!(envelope.text.as_deref(), Some("hi"));
        assert_eq!(envelope.from.expect("from").id, "U1");
        assert_eq!(envelope.to[0].id, "C1");
    }

    #[test]
    fn webhook_drops_retries_and_bot_messages_without_events() {
        let mut headers = Map::new();
        headers.insert("X-Slack-Retry-Num".to_string(), Value::String("1".into()));
        let retry = normalize_body(
            &headers,
            r#"{"type":"event_callback","event":{"text":"hi","channel":"C1","user":"U1"}}"#,
        )
        .expect("retry");
        let parsed: Value = serde_json::from_str(&retry).expect("json");
        assert_eq!(parsed["events"].as_array().expect("events").len(), 0);

        let bot = normalize_body(
            &Map::new(),
            r#"{"type":"event_callback","event":{"text":"hi","channel":"C1","bot_id":"B1"}}"#,
        )
        .expect("bot");
        let parsed: Value = serde_json::from_str(&bot).expect("json");
        assert_eq!(parsed["events"].as_array().expect("events").len(), 0);
    }

    #[test]
    fn webhook_block_action_uses_channel_object_id_for_reply_route() {
        let body = json!({
            "type": "block_actions",
            "channel": {"id": "C1", "name": "helpdesk"},
            "user": {"id": "U1"},
            "actions": [{
                "action_id": "approve",
                "value": serde_json::to_string(&json!({
                    "routeToCardId": "next-card",
                    "decision": "approve",
                    "count": 2
                })).expect("action value")
            }]
        });

        let out = normalize_body(&Map::new(), &body.to_string()).expect("normalized");
        let parsed: Value = serde_json::from_str(&out).expect("json");
        let envelope: greentic_types::ChannelMessageEnvelope =
            serde_json::from_value(parsed["events"][0].clone()).expect("channel envelope");

        assert_eq!(envelope.text.as_deref(), Some("[card:next-card]"));
        assert_eq!(envelope.channel, "C1");
        assert_eq!(envelope.session_id, "C1");
        assert_eq!(envelope.to[0].id, "C1");
        assert_eq!(envelope.from.expect("from").id, "U1");
        assert_eq!(envelope.metadata["routeToCardId"], "next-card");
        assert_eq!(envelope.metadata["decision"], "approve");
        assert_eq!(envelope.metadata["count"], "2");
        assert_eq!(envelope.metadata["slack.action_id"], "approve");
    }

    #[test]
    fn webhook_normalizes_app_home_opened_as_user_entered() {
        let body = json!({
            "type": "event_callback",
            "team_id": "T1",
            "event": {
                "type": "app_home_opened",
                "user": "U1",
                "event_ts": "1780414157.651"
            }
        });

        let out = normalize_body(&Map::new(), &body.to_string()).expect("normalized");
        let parsed: Value = serde_json::from_str(&out).expect("json");
        let envelope: greentic_types::ChannelMessageEnvelope =
            serde_json::from_value(parsed["events"][0].clone()).expect("channel envelope");

        assert_eq!(envelope.session_id, "U1");
        assert_eq!(envelope.metadata["event_type"], "channel.user.entered");
        assert_eq!(envelope.metadata["autoStart"], "true");
        assert_eq!(envelope.metadata["provider"], "slack");
        assert_eq!(envelope.metadata["reason"], "app_home_opened");
        assert_eq!(
            envelope.metadata["idempotency_key"],
            "lifecycle.user_entered:slack:T1:U1:U1:app_home_opened"
        );
    }

    #[test]
    fn webhook_normalizes_member_joined_channel_as_user_entered() {
        let body = json!({
            "type": "event_callback",
            "team_id": "T1",
            "event": {
                "type": "member_joined_channel",
                "channel": "C1",
                "user": "U1",
                "event_ts": "1780414157.652"
            }
        });

        let out = normalize_body(&Map::new(), &body.to_string()).expect("normalized");
        let parsed: Value = serde_json::from_str(&out).expect("json");
        let envelope: greentic_types::ChannelMessageEnvelope =
            serde_json::from_value(parsed["events"][0].clone()).expect("channel envelope");

        assert_eq!(envelope.session_id, "C1");
        assert_eq!(envelope.metadata["event_type"], "channel.user.entered");
        assert_eq!(envelope.metadata["reason"], "member_joined_channel");
        assert_eq!(envelope.metadata["channel_id"], "C1");
        assert_eq!(
            envelope.metadata["idempotency_key"],
            "lifecycle.user_entered:slack:T1:C1:U1:member_joined_channel"
        );
    }

    #[test]
    fn webhook_returns_url_verification_challenge() {
        let out = normalize_body(
            &Map::new(),
            r#"{"type":"url_verification","challenge":"abc123"}"#,
        )
        .expect("challenge");
        let parsed: Value = serde_json::from_str(&out).expect("json");

        assert_eq!(parsed["status"], 200);
        assert_eq!(parsed["body"], "abc123");
        assert_eq!(parsed["events"].as_array().expect("events").len(), 0);
    }
}
