//! Inbound Slack events (`ingest_http`).
//!
//! Handles:
//! - Slack URL verification (responds with the challenge string)
//! - Interactive payloads (`block_actions`) — either trigger a modal or
//!   produce a channel envelope
//! - `view_submission` (modal results) — delegates to [`super::modal`]
//! - Slack Events API (`event_callback`) and flat fallback payloads
//! - Drops Slack retries (`X-Slack-Retry-Num` header) to avoid duplicates
//! - Skips bot-authored messages to prevent echo loops

use base64::{Engine as _, engine::general_purpose::STANDARD};
use greentic_types::messaging::universal_dto::{HttpInV1, HttpOutV1};
use provider_common::http_compat::{http_out_error, http_out_v1_bytes, parse_operator_http_in};
use serde_json::{Value, json};

use super::modal::{handle_view_submission, open_slack_modal};
use super::{build_slack_envelope, mark_slack_user_entered, set_verified_caller};
use crate::DEFAULT_SIGNING_SECRET_KEY;
use crate::bindings::greentic::http::http_client as client;
use crate::config::{ProviderConfig, get_secret_string, load_config, resolve_bot_token};

/// Decide whether this request provably came from Slack.
///
/// A verified caller is an identity claim, so it is only ever stamped when the
/// provider itself checked `X-Slack-Signature` (HMAC-SHA256 over
/// `v0:{timestamp}:{raw body}`, 300 s replay window) against the signing
/// secret stored at setup. The same `slack_auth_core::verify_request` runs in
/// `messaging-ingress-slack`, the component greentic-start's webhook gate
/// uses; this check does not rely on that gate having run.
///
/// Fails closed: no secret, no header, a stale timestamp or a mismatch all
/// read as "not authentic". It never rejects the request: the host gate owns
/// refusal, and an unauthenticated delivery keeps flowing as an anonymous one.
fn request_is_authentic(
    request: &HttpInV1,
    body: &[u8],
    signing_secret: &dyn Fn() -> Option<String>,
    now_secs: i64,
) -> bool {
    let header = |name: &str| {
        request
            .headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case(name))
            .map(|h| h.value.as_str())
    };
    let (Some(signature), Some(timestamp)) = (
        header("x-slack-signature"),
        header("x-slack-request-timestamp"),
    ) else {
        return false;
    };
    // Read the secret only for a request that claims to be signed.
    let Some(secret) = signing_secret().filter(|s| !s.is_empty()) else {
        return false;
    };
    slack_auth_core::verify_request(&secret, signature, timestamp, body, now_secs).is_ok()
}

pub(crate) fn ingest_http(input_json: &[u8]) -> Vec<u8> {
    ingest_http_at(
        input_json,
        &|| get_secret_string(DEFAULT_SIGNING_SECRET_KEY).ok(),
        slack_auth_core::now_unix_secs(),
    )
}

/// [`ingest_http`] with the signing secret and the clock injected.
fn ingest_http_at(
    input_json: &[u8],
    signing_secret: &dyn Fn() -> Option<String>,
    now_secs: i64,
) -> Vec<u8> {
    // Try native greentic-types format first, fall back to operator format
    let request = match serde_json::from_slice::<HttpInV1>(input_json) {
        Ok(req) => req,
        Err(_) => match parse_operator_http_in(input_json) {
            Ok(req) => req,
            Err(err) => return http_out_error(400, &format!("invalid http input: {err}")),
        },
    };
    let body_bytes = match STANDARD.decode(&request.body_b64) {
        Ok(bytes) => bytes,
        Err(err) => return http_out_error(400, &format!("invalid body encoding: {err}")),
    };
    let body_val: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    let authentic = request_is_authentic(&request, &body_bytes, signing_secret, now_secs);

    // Slack URL verification challenge — must respond with the challenge value.
    // Sent when setting Event Subscriptions or Interactivity Request URL.
    if body_val.get("type").and_then(Value::as_str) == Some("url_verification") {
        let challenge = body_val
            .get("challenge")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let out = HttpOutV1 {
            status: 200,
            headers: vec![],
            body_b64: STANDARD.encode(challenge.as_bytes()),
            events: vec![],
        };
        return http_out_v1_bytes(&out);
    }

    // Slack interactive payloads (button clicks) come as URL-encoded `payload=<json>`
    // or directly as JSON with `type: "block_actions"`.
    // Also check for URL-encoded form body.
    let interactive_payload =
        if body_val.get("type").and_then(Value::as_str) == Some("block_actions") {
            Some(body_val.clone())
        } else {
            // Try URL-encoded: body may be "payload=%7B..." raw text.
            let body_str = String::from_utf8(body_bytes.clone()).unwrap_or_default();
            if let Some(payload) = body_str.strip_prefix("payload=") {
                let decoded = urldecode(payload);
                serde_json::from_str::<Value>(&decoded)
                    .ok()
                    .filter(|v| v.get("type").and_then(Value::as_str) == Some("block_actions"))
            } else {
                None
            }
        };

    // Handle view_submission — Slack modal form submitted.
    let view_submission_payload =
        if body_val.get("type").and_then(Value::as_str) == Some("view_submission") {
            Some(body_val.clone())
        } else {
            let body_str = String::from_utf8(body_bytes.clone()).unwrap_or_default();
            if let Some(payload) = body_str.strip_prefix("payload=") {
                let decoded = urldecode(payload);
                serde_json::from_str::<Value>(&decoded)
                    .ok()
                    .filter(|v| v.get("type").and_then(Value::as_str) == Some("view_submission"))
            } else {
                None
            }
        };

    if let Some(submission) = view_submission_payload {
        return handle_view_submission(&submission, authentic);
    }

    if let Some(interactive) = interactive_payload {
        // The provider config rides on the ingest input, not on Slack's payload.
        let host_input: Value = serde_json::from_slice(input_json).unwrap_or(Value::Null);
        return handle_block_actions(&interactive, &host_input, authentic);
    }

    // Drop Slack retries — only process the first delivery.
    if is_slack_retry(&request) {
        let out = HttpOutV1 {
            status: 200,
            headers: Vec::new(),
            body_b64: STANDARD.encode(b"ok"),
            events: vec![],
        };
        return http_out_v1_bytes(&out);
    }

    // Slack Events API: {"type":"event_callback","event":{...}}
    // Legacy/generic:   {"body":{...}}
    // Flat:             {"text":"...","channel":"..."}
    let payload = body_val
        .get("event")
        .or_else(|| body_val.get("body"))
        .cloned()
        .unwrap_or_else(|| body_val.clone());

    // Skip bot messages to prevent echo loops.
    if is_bot_message(&payload) {
        let out = HttpOutV1 {
            status: 200,
            headers: Vec::new(),
            body_b64: STANDARD.encode(b"ok"),
            events: vec![],
        };
        return http_out_v1_bytes(&out);
    }

    let team_id = slack_team_id(&body_val);
    if let Some(envelope) = lifecycle_envelope_from_payload(&payload, team_id.as_deref()) {
        let normalized = json!({
            "ok": true,
            "event": body_val,
            "channel": envelope.metadata.get("channel_id").cloned(),
        });
        let normalized_bytes = serde_json::to_vec(&normalized).unwrap_or_else(|_| b"{}".to_vec());
        let out = HttpOutV1 {
            status: 200,
            headers: Vec::new(),
            body_b64: STANDARD.encode(&normalized_bytes),
            events: vec![envelope],
        };
        return http_out_v1_bytes(&out);
    }

    let text = payload
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let channel = payload
        .get("channel")
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    let sender = payload
        .get("user")
        .or_else(|| payload.get("user_id"))
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    // Fetch user locale from Slack API (users.info) for i18n card translation.
    let slack_cfg = load_config(&body_val).ok();
    let user_locale = sender.as_deref().and_then(|uid| {
        slack_cfg
            .as_ref()
            .and_then(|c| fetch_slack_user_locale(c, uid))
    });
    let caller = authentic
        .then(|| slack_auth_core::caller_for_event(&body_val))
        .flatten();
    let mut envelope = build_slack_envelope(text, channel.clone(), sender);
    set_verified_caller(&mut envelope, caller);
    if let Some(locale) = user_locale {
        envelope.metadata.insert("locale".to_string(), locale);
    }
    let normalized = json!({
        "ok": true,
        "event": body_val,
        "channel": channel,
    });
    let normalized_bytes = serde_json::to_vec(&normalized).unwrap_or_else(|_| b"{}".to_vec());
    let out = HttpOutV1 {
        status: 200,
        headers: Vec::new(),
        body_b64: STANDARD.encode(&normalized_bytes),
        events: vec![envelope],
    };
    http_out_v1_bytes(&out)
}

fn lifecycle_envelope_from_payload(
    payload: &Value,
    team_id: Option<&str>,
) -> Option<greentic_types::ChannelMessageEnvelope> {
    let event_type = payload.get("type").and_then(Value::as_str)?;
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
    let mut envelope = build_slack_envelope(
        String::new(),
        Some(session_id),
        user_id.map(ToOwned::to_owned),
    );
    mark_slack_user_entered(&mut envelope, reason, team_id, channel_id, user_id);
    if let Some(event_ts) = payload.get("event_ts").and_then(Value::as_str) {
        envelope.id = format!("slack:{event_ts}");
        envelope
            .metadata
            .insert("event_ts".to_string(), event_ts.to_string());
    }
    Some(envelope)
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

/// Process a Slack `block_actions` interaction (button click).
///
/// If the button is flagged as a modal trigger we open a Slack modal via
/// `views.open`; otherwise we produce a channel envelope carrying the
/// action's metadata for downstream routing.
fn handle_block_actions(interactive: &Value, host_input: &Value, authentic: bool) -> Vec<u8> {
    // Approval decisions are routed first: the generic path below forwards every
    // action-value key into envelope metadata, which would splat the decision
    // token across telemetry.
    if super::approval::is_approval_interaction(interactive) {
        return super::approval::handle_approval_interaction(interactive, host_input);
    }

    let actions = interactive
        .get("actions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let channel = interactive
        .get("channel")
        .and_then(|v| v.get("id"))
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    let sender = interactive
        .get("user")
        .and_then(|v| v.get("id"))
        .and_then(Value::as_str)
        .map(|s| s.to_string());

    // Extract routing info from first action's value.
    let first_action = actions.first().cloned().unwrap_or(Value::Null);
    let action_value_str = first_action
        .get("value")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let action_id = first_action
        .get("action_id")
        .and_then(Value::as_str)
        .unwrap_or_default();

    // Check if this button triggers a modal (AC input fields present).
    let parsed_action_val = serde_json::from_str::<Value>(action_value_str).ok();
    let is_modal = parsed_action_val
        .as_ref()
        .and_then(|v| v.get("ac_modal"))
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if is_modal {
        let trigger_id = interactive
            .get("trigger_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !trigger_id.is_empty() {
            // Retrieve input specs from message metadata (not button value).
            let msg_metadata_inputs = interactive
                .get("message")
                .and_then(|m| m.get("metadata"))
                .and_then(|m| m.get("event_payload"))
                .and_then(|p| p.get("inputs"))
                .cloned()
                .unwrap_or(json!([]));
            // Merge: action data + input specs for the modal builder.
            let mut modal_data = parsed_action_val.clone().unwrap_or(json!({}));
            if let Some(obj) = modal_data.as_object_mut() {
                obj.insert("ac_modal_inputs".into(), msg_metadata_inputs);
            }
            return open_slack_modal(trigger_id, &modal_data, channel.as_deref());
        }
    }

    // Try to parse action value as JSON for routeToCardId.
    let (_route_to_card, _card_id, action_text) =
        if let Ok(val) = serde_json::from_str::<Value>(action_value_str) {
            let rtc = val
                .get("routeToCardId")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let cid = val
                .get("cardId")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let text = if !rtc.is_empty() {
                format!("[card:{rtc}]")
            } else if !cid.is_empty() {
                format!("[action:{cid}]")
            } else {
                format!("[action:{action_id}]")
            };
            (rtc, cid, text)
        } else {
            (
                String::new(),
                String::new(),
                format!("[action:{action_id}]"),
            )
        };

    let caller = authentic
        .then(|| slack_auth_core::caller_for_block_actions(interactive))
        .flatten();
    let mut envelope = build_slack_envelope(action_text, channel.clone(), sender);
    set_verified_caller(&mut envelope, caller);
    // Forward ALL Action.Submit data fields to metadata for MCP routing.
    if let Ok(val) = serde_json::from_str::<Value>(action_value_str)
        && let Some(obj) = val.as_object()
    {
        for (k, v) in obj {
            let s = match v {
                Value::String(s) => s.clone(),
                _ => v.to_string(),
            };
            envelope.metadata.insert(k.clone(), s);
        }
    }
    envelope
        .metadata
        .insert("slack.action_id".into(), action_id.to_string());
    envelope
        .metadata
        .insert("slack.action_value".into(), action_value_str.to_string());

    let normalized = json!({
        "ok": true,
        "event": interactive,
        "channel": channel,
    });
    let normalized_bytes = serde_json::to_vec(&normalized).unwrap_or_else(|_| b"{}".to_vec());
    let out = HttpOutV1 {
        status: 200,
        headers: Vec::new(),
        body_b64: STANDARD.encode(&normalized_bytes),
        events: vec![envelope],
    };
    http_out_v1_bytes(&out)
}

/// Fetch user locale from Slack `users.info` API.
fn fetch_slack_user_locale(cfg: &ProviderConfig, user_id: &str) -> Option<String> {
    let token = resolve_bot_token(cfg);
    if token.is_empty() {
        return None;
    }
    let api_base = cfg
        .api_base_url
        .as_deref()
        .unwrap_or("https://slack.com/api");
    let url = format!("{api_base}/users.info?user={user_id}");
    let request = client::Request {
        method: "GET".into(),
        url,
        headers: vec![("Authorization".into(), format!("Bearer {token}"))],
        body: None,
    };
    let resp = client::send(&request, None, None).ok()?;
    if resp.status < 200 || resp.status >= 300 {
        return None;
    }
    let body: Value = serde_json::from_slice(&resp.body.unwrap_or_default()).ok()?;
    body.get("user")
        .and_then(|u| u.get("locale"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Check if the request is a Slack retry (X-Slack-Retry-Num header present).
fn is_slack_retry(request: &HttpInV1) -> bool {
    request.headers.iter().any(|h| {
        h.name.eq_ignore_ascii_case("x-slack-retry-num")
            || h.name.eq_ignore_ascii_case("X-Slack-Retry-Num")
    })
}

/// Check if the event payload is from a bot (prevents echo loops).
fn is_bot_message(payload: &Value) -> bool {
    // bot_id field present on bot-authored messages
    if payload.get("bot_id").is_some_and(|v| !v.is_null()) {
        return true;
    }
    // subtype "bot_message" is another indicator
    if payload.get("subtype").and_then(Value::as_str) == Some("bot_message") {
        return true;
    }
    false
}

/// Simple percent-decode for URL-encoded strings.
pub(super) fn urldecode(input: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;
    const SECRET: &str = "test-slack-signing-secret";

    fn request(body: &[u8], headers: Value) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "method": "POST",
            "path": "/slack",
            "query": [],
            "headers": headers,
            "body_b64": STANDARD.encode(body)
        }))
        .expect("request")
    }

    /// Native tests have no secret store: the unauthenticated path.
    fn ingest_http(input_json: &[u8]) -> Vec<u8> {
        ingest_http_at(input_json, &|| None, NOW)
    }

    fn parse_out(bytes: Vec<u8>) -> HttpOutV1 {
        serde_json::from_slice(&bytes).expect("http out")
    }

    fn decoded_body(out: &HttpOutV1) -> Value {
        let body = STANDARD.decode(&out.body_b64).expect("body");
        serde_json::from_slice(&body).unwrap_or_else(|_| json!(String::from_utf8_lossy(&body)))
    }

    #[test]
    fn ingest_rejects_bad_http_and_bad_body_encoding() {
        let bad_http = parse_out(ingest_http(b"{"));
        assert_eq!(bad_http.status, 400);

        let bad_body = parse_out(ingest_http(
            json!({
                "method": "POST",
                "path": "/slack",
                "query": [],
                "headers": [],
                "body_b64": "not base64"
            })
            .to_string()
            .as_bytes(),
        ));
        assert_eq!(bad_body.status, 400);
    }

    #[test]
    fn ingest_drops_retries_and_bot_messages_without_events() {
        let retry = parse_out(ingest_http(&request(
            br#"{"event":{"text":"hello","channel":"C1","user":"U1"}}"#,
            json!([{"name":"X-Slack-Retry-Num","value":"1"}]),
        )));
        assert_eq!(retry.status, 200);
        assert!(retry.events.is_empty());
        assert_eq!(decoded_body(&retry), json!("ok"));

        let bot = parse_out(ingest_http(&request(
            br#"{"event":{"text":"hello","channel":"C1","bot_id":"B1"}}"#,
            json!([]),
        )));
        assert_eq!(bot.status, 200);
        assert!(bot.events.is_empty());
        assert_eq!(decoded_body(&bot), json!("ok"));
    }

    #[test]
    fn ingest_event_callback_produces_envelope() {
        let out = parse_out(ingest_http(&request(
            br#"{"type":"event_callback","event":{"text":"hello","channel":"C1","user":"U1"}}"#,
            json!([]),
        )));
        let body = decoded_body(&out);

        assert_eq!(out.status, 200);
        assert_eq!(body["ok"], true);
        assert_eq!(body["channel"], "C1");
        assert_eq!(out.events.len(), 1);
        assert_eq!(out.events[0].text.as_deref(), Some("hello"));
        assert_eq!(out.events[0].metadata["channel"], "C1");
    }

    #[test]
    fn ingest_app_home_opened_produces_user_entered_envelope() {
        let out = parse_out(ingest_http(&request(
            br#"{"type":"event_callback","team_id":"T1","event":{"type":"app_home_opened","user":"U1","event_ts":"1780414157.651"}}"#,
            json!([]),
        )));

        assert_eq!(out.status, 200);
        assert_eq!(out.events.len(), 1);
        let event = &out.events[0];
        assert_eq!(event.session_id, "U1");
        assert_eq!(
            event.metadata.get("event_type").map(String::as_str),
            Some("channel.user.entered")
        );
        assert_eq!(
            event.metadata.get("autoStart").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            event.metadata.get("provider").map(String::as_str),
            Some("slack")
        );
        assert_eq!(
            event.metadata.get("reason").map(String::as_str),
            Some("app_home_opened")
        );
        assert_eq!(
            event.metadata.get("idempotency_key").map(String::as_str),
            Some("lifecycle.user_entered:slack:T1:U1:U1:app_home_opened")
        );
    }

    #[test]
    fn ingest_member_joined_channel_produces_user_entered_envelope() {
        let out = parse_out(ingest_http(&request(
            br#"{"type":"event_callback","team_id":"T1","event":{"type":"member_joined_channel","channel":"C1","user":"U1","event_ts":"1780414157.652"}}"#,
            json!([]),
        )));

        assert_eq!(out.status, 200);
        assert_eq!(out.events.len(), 1);
        let event = &out.events[0];
        assert_eq!(event.session_id, "C1");
        assert_eq!(
            event.metadata.get("event_type").map(String::as_str),
            Some("channel.user.entered")
        );
        assert_eq!(
            event.metadata.get("reason").map(String::as_str),
            Some("member_joined_channel")
        );
        assert_eq!(
            event.metadata.get("channel_id").map(String::as_str),
            Some("C1")
        );
        assert_eq!(
            event.metadata.get("idempotency_key").map(String::as_str),
            Some("lifecycle.user_entered:slack:T1:C1:U1:member_joined_channel")
        );
    }

    #[test]
    fn block_actions_forward_submit_data_to_metadata() {
        let payload = json!({
            "type": "block_actions",
            "channel": {"id": "C1"},
            "user": {"id": "U1"},
            "actions": [{
                "action_id": "approve",
                "value": serde_json::to_string(&json!({
                    "routeToCardId": "card-1",
                    "decision": "approve",
                    "count": 2
                })).expect("action value")
            }]
        });

        let out = parse_out(ingest_http(&request(
            payload.to_string().as_bytes(),
            json!([]),
        )));

        assert_eq!(out.status, 200);
        assert_eq!(out.events.len(), 1);
        assert_eq!(out.events[0].text.as_deref(), Some("[card:card-1]"));
        assert_eq!(out.events[0].metadata["decision"], "approve");
        assert_eq!(out.events[0].metadata["count"], "2");
        assert_eq!(out.events[0].metadata["slack.action_id"], "approve");
    }

    #[test]
    fn approval_clicks_take_the_approval_path_not_the_generic_one() {
        let token = "EXAMPLE-TOKEN-NOT-A-REAL-SECRET";
        let payload = json!({
            "type": "block_actions",
            "channel": {"id": "C1"},
            "user": {"id": "U1"},
            "actions": [{
                "action_id": "greentic_approval_approve",
                "value": json!({"v": 1, "cid": "default::run=RUN-1::node=gate", "tok": token})
                    .to_string()
            }]
        });

        let out = parse_out(ingest_http(&request(
            payload.to_string().as_bytes(),
            json!([]),
        )));

        assert_eq!(out.status, 200);
        assert_eq!(out.events.len(), 1);
        let event = &out.events[0];
        assert_eq!(
            event.correlation_id.as_deref(),
            Some("default::run=RUN-1::node=gate")
        );
        assert!(event.extensions.contains_key("greentic.approval.response"));
        for (key, value) in &event.metadata {
            assert!(!value.contains(token), "token leaked into metadata[{key}]");
        }

        let body = STANDARD.decode(&out.body_b64).expect("body");
        let body = String::from_utf8(body).expect("utf8");
        assert!(body.contains("replace_original"));
        assert!(!body.contains(token));
    }

    // --- verified caller -------------------------------------------------

    fn sign(secret: &str, ts: i64, body: &[u8]) -> String {
        use hmac::{Hmac, KeyInit, Mac};
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).expect("hmac");
        mac.update(format!("v0:{ts}:").as_bytes());
        mac.update(body);
        let hex: String = mac
            .finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        format!("v0={hex}")
    }

    /// Run `body` through ingest as Slack would deliver it, signed with `key`
    /// at `ts`, while the provider holds `stored` as its signing secret.
    fn deliver(body: &Value, key: &str, ts: i64, stored: Option<&str>) -> HttpOutV1 {
        let raw = body.to_string();
        let headers = json!([
            {"name": "X-Slack-Signature", "value": sign(key, ts, raw.as_bytes())},
            {"name": "X-Slack-Request-Timestamp", "value": ts.to_string()}
        ]);
        let stored = stored.map(str::to_string);
        parse_out(ingest_http_at(
            &request(raw.as_bytes(), headers),
            &move || stored.clone(),
            NOW,
        ))
    }

    fn dm() -> Value {
        json!({
            "type": "event_callback",
            "team_id": "T111",
            "event": {"type": "message", "channel": "D999", "channel_type": "im",
                      "user": "U123ABC", "text": "hello"}
        })
    }

    fn caller_of(out: &HttpOutV1) -> Option<Value> {
        assert_eq!(out.events.len(), 1, "exactly one envelope");
        out.events[0].extensions.get("caller").cloned()
    }

    #[test]
    fn a_signed_dm_is_stamped_with_the_verified_caller() {
        let out = deliver(&dm(), SECRET, NOW, Some(SECRET));
        assert_eq!(
            caller_of(&out),
            Some(json!({"user_verified": true, "sub": "U123ABC", "iss": "slack:T111"}))
        );
    }

    #[test]
    fn no_caller_unless_the_provider_verified_the_signature() {
        // Wrong secret, stale, future, missing secret, empty secret.
        assert_eq!(caller_of(&deliver(&dm(), "other", NOW, Some(SECRET))), None);
        assert_eq!(
            caller_of(&deliver(&dm(), SECRET, NOW - 301, Some(SECRET))),
            None
        );
        assert_eq!(
            caller_of(&deliver(&dm(), SECRET, NOW + 301, Some(SECRET))),
            None
        );
        assert_eq!(caller_of(&deliver(&dm(), SECRET, NOW, None)), None);
        assert_eq!(caller_of(&deliver(&dm(), SECRET, NOW, Some(""))), None);

        // No signature headers at all.
        let unsigned = parse_out(ingest_http_at(
            &request(dm().to_string().as_bytes(), json!([])),
            &|| Some(SECRET.to_string()),
            NOW,
        ));
        assert_eq!(caller_of(&unsigned), None);

        // Signature over a different body.
        let raw = dm().to_string();
        let headers = json!([
            {"name": "x-slack-signature", "value": sign(SECRET, NOW, b"{}")},
            {"name": "x-slack-request-timestamp", "value": NOW.to_string()}
        ]);
        let forged = parse_out(ingest_http_at(
            &request(raw.as_bytes(), headers),
            &|| Some(SECRET.to_string()),
            NOW,
        ));
        assert_eq!(caller_of(&forged), None);
    }

    #[test]
    fn a_sender_written_caller_never_survives() {
        let mut body = dm();
        body["event"]["extensions"] = json!({"caller": {"user_verified": true, "sub": "U999"}});
        let out = deliver(&body, "other", NOW, Some(SECRET));
        assert_eq!(caller_of(&out), None);
    }

    #[test]
    fn channels_groups_and_multiparty_dms_get_no_caller() {
        for (channel, ty) in [("C999", "channel"), ("G999", "group"), ("G998", "mpim")] {
            let mut body = dm();
            body["event"]["channel"] = json!(channel);
            body["event"]["channel_type"] = json!(ty);
            let out = deliver(&body, SECRET, NOW, Some(SECRET));
            assert_eq!(caller_of(&out), None, "{channel}/{ty}");
        }
    }

    #[test]
    fn bot_events_get_no_caller_and_no_envelope() {
        let mut body = dm();
        body["event"]["bot_id"] = json!("B1");
        let out = deliver(&body, SECRET, NOW, Some(SECRET));
        assert!(out.events.is_empty(), "bot messages are dropped");
    }

    #[test]
    fn grid_and_connect_select_the_issuer() {
        let mut grid = dm();
        grid["enterprise_id"] = json!("E555");
        let out = deliver(&grid, SECRET, NOW, Some(SECRET));
        assert_eq!(caller_of(&out).unwrap()["iss"], "slack:E555");

        let mut connect = dm();
        connect["event"]["user_team"] = json!("T222");
        let out = deliver(&connect, SECRET, NOW, Some(SECRET));
        assert_eq!(caller_of(&out).unwrap()["iss"], "slack:T222");
    }

    #[test]
    fn malformed_ids_are_omitted_not_sent() {
        let mut body = dm();
        body["event"]["user"] = json!("not a slack id");
        let out = deliver(&body, SECRET, NOW, Some(SECRET));
        assert_eq!(caller_of(&out), None);
        let mut body = dm();
        body["team_id"] = json!("bad team");
        let out = deliver(&body, SECRET, NOW, Some(SECRET));
        assert_eq!(caller_of(&out), None);
    }

    #[test]
    fn a_signed_dm_block_action_is_stamped_and_a_channel_one_is_not() {
        let click = |channel: &str| {
            json!({
                "type": "block_actions",
                "user": {"id": "U123ABC", "team_id": "T111"},
                "team": {"id": "T111"},
                "channel": {"id": channel},
                "actions": [{"action_id": "go", "value": "{\"routeToCardId\":\"c2\"}"}]
            })
        };
        let out = deliver(&click("D999"), SECRET, NOW, Some(SECRET));
        assert_eq!(
            caller_of(&out),
            Some(json!({"user_verified": true, "sub": "U123ABC", "iss": "slack:T111"}))
        );
        assert_eq!(
            caller_of(&deliver(&click("C999"), SECRET, NOW, Some(SECRET))),
            None
        );
        assert_eq!(
            caller_of(&deliver(&click("D999"), "other", NOW, Some(SECRET))),
            None
        );
    }

    #[test]
    fn a_signed_dm_view_submission_is_stamped_and_a_channel_one_is_not() {
        let submit = |channel: &str| {
            json!({
                "type": "view_submission",
                "user": {"id": "U123ABC", "team_id": "T111"},
                "team": {"id": "T111"},
                "view": {
                    "private_metadata": json!({"_channel": channel}).to_string(),
                    "state": {"values": {}}
                }
            })
        };
        let out = deliver(&submit("D999"), SECRET, NOW, Some(SECRET));
        assert_eq!(
            caller_of(&out),
            Some(json!({"user_verified": true, "sub": "U123ABC", "iss": "slack:T111"}))
        );
        assert_eq!(
            caller_of(&deliver(&submit("C999"), SECRET, NOW, Some(SECRET))),
            None
        );
        assert_eq!(
            caller_of(&deliver(&submit("D999"), "other", NOW, Some(SECRET))),
            None
        );
    }

    #[test]
    fn urldecode_handles_plus_valid_and_invalid_escapes() {
        assert_eq!(urldecode("hello+world%21"), "hello world!");
        assert_eq!(urldecode("bad%zz"), "bad%zz");
    }
}
