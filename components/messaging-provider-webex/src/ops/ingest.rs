//! Inbound Webex webhook ingestion.
//!
//! `ingest_http` accepts either the native `HttpInV1` or the operator wrapper
//! format, normalises the body, and dispatches to [`handle_webhook_event`] to
//! produce a [`ChannelMessageEnvelope`]. The actual Webex API calls (fetch
//! message / fetch attachment actions) and envelope helpers live in
//! [`super::ingest_helpers`] to keep both files under the 500-line cap.

use base64::{Engine, engine::general_purpose::STANDARD};
use greentic_types::ChannelMessageEnvelope;
use greentic_types::messaging::universal_dto::{HttpInV1, HttpOutV1};
use hmac::{Hmac, KeyInit, Mac};
use provider_common::http_compat::{http_out_error, http_out_v1_bytes, parse_operator_http_in};
use provider_common::lifecycle_events::{mark_user_entered, user_entered_idempotency_key};
use provider_common::redact;
use provider_common::telemetry::{self, Field, Level, event, field};
use serde_json::{Value, json};
// Webex webhook signatures are defined as HMAC-SHA1 by the provider API.
// foxguard: ignore[rs/no-weak-hash]
use sha1::Sha1;

use super::ingest_helpers::{
    build_webhook_envelope, build_webhook_metadata, envelope_from_details, fetch_action_details,
    fetch_message_details, pick_sender,
};
#[cfg(not(test))]
use crate::DEFAULT_WEBHOOK_SECRET_KEY;
use crate::config::{get_secret_string, load_config};
use crate::{DEFAULT_API_BASE, DEFAULT_TOKEN_KEY, PROVIDER_TYPE, ProviderConfig};

pub(crate) struct IngestOutcome {
    pub(crate) envelope: Option<ChannelMessageEnvelope>,
    pub(crate) status: u16,
    pub(crate) error: Option<String>,
}

pub(crate) fn ingest_http(input_json: &[u8]) -> Vec<u8> {
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
    let cfg = load_config(&json!({})).unwrap_or_default();
    if let Some(secret) = cfg
        .webhook_secret
        .clone()
        .or_else(resolve_webhook_secret_for_verification)
        && !verify_webex_signature(&request.headers, &body_bytes, &secret)
    {
        return http_out_error(401, "invalid Webex webhook signature");
    }
    let body_val: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    let outcome = handle_webhook_event(&body_val, &cfg);

    let mut normalized = json!({
        "ok": outcome.error.is_none(),
        "event": body_val,
    });
    if let Some(err) = &outcome.error {
        normalized
            .as_object_mut()
            .map(|map| map.insert("error".into(), Value::String(err.clone())));
    }

    let normalized_bytes = serde_json::to_vec(&normalized).unwrap_or_else(|_| b"{}".to_vec());
    let out = HttpOutV1 {
        status: outcome.status,
        headers: Vec::new(),
        body_b64: STANDARD.encode(&normalized_bytes),
        events: outcome.envelope.into_iter().collect(),
    };
    http_out_v1_bytes(&out)
}

#[cfg(not(test))]
fn resolve_webhook_secret_for_verification() -> Option<String> {
    get_secret_string(DEFAULT_WEBHOOK_SECRET_KEY).ok()
}

#[cfg(test)]
fn resolve_webhook_secret_for_verification() -> Option<String> {
    None
}

fn verify_webex_signature(
    headers: &[greentic_types::messaging::universal_dto::Header],
    body: &[u8],
    secret: &str,
) -> bool {
    let Some(expected) = find_header_value(headers, "x-spark-signature")
        .or_else(|| find_header_value(headers, "x-webex-signature"))
    else {
        return false;
    };
    let Some(actual) = hmac_sha1_hex(secret.as_bytes(), body) else {
        return false;
    };
    constant_time_eq_hex(&actual, expected.trim())
}

fn find_header_value(
    headers: &[greentic_types::messaging::universal_dto::Header],
    key: &str,
) -> Option<String> {
    headers
        .iter()
        .find(|header| header.name.eq_ignore_ascii_case(key))
        .map(|header| header.value.clone())
}

fn hmac_sha1_hex(secret: &[u8], body: &[u8]) -> Option<String> {
    // Webex signs webhook payloads with HMAC-SHA1; changing this breaks provider verification.
    // foxguard: ignore[rs/no-weak-hash]
    let mut mac = Hmac::<Sha1>::new_from_slice(secret).ok()?;
    mac.update(body);
    Some(hex_lower(&mac.finalize().into_bytes()))
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

fn constant_time_eq_hex(actual: &str, expected: &str) -> bool {
    let actual = actual.as_bytes();
    let expected = expected.as_bytes();
    if actual.len() != expected.len() {
        return false;
    }
    actual
        .iter()
        .zip(expected)
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

pub(crate) fn handle_webhook_event(body: &Value, cfg: &ProviderConfig) -> IngestOutcome {
    let resource = body
        .get("resource")
        .and_then(|s| s.as_str())
        .unwrap_or_default();
    let event = body
        .get("event")
        .and_then(|s| s.as_str())
        .unwrap_or_default();
    let data = body.get("data").unwrap_or(&Value::Null);
    let message_id = data
        .get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let webhook_room = data
        .get("roomId")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let webhook_person_email = data
        .get("personEmail")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let webhook_person_id = data
        .get("personId")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    if resource == "messages"
        && event == "created"
        && webhook_person_email
            .as_deref()
            .is_some_and(is_webex_bot_email)
    {
        let redacted_sender = webhook_person_email
            .as_deref()
            .map(redact::user_id)
            .unwrap_or_default();
        telemetry::emit(
            Level::Info,
            PROVIDER_TYPE,
            "webex ignored bot-authored message webhook",
            &[
                Field {
                    key: field::USER,
                    value: &redacted_sender,
                },
                Field {
                    key: field::MESSAGE_ID,
                    value: message_id.as_deref().unwrap_or_default(),
                },
            ],
        );
        return IngestOutcome {
            envelope: None,
            status: 200,
            error: None,
        };
    }

    if resource == "memberships" && event == "created" {
        let membership_id = message_id.clone();
        let session_id = webhook_room
            .clone()
            .unwrap_or_else(|| membership_id.clone().unwrap_or_else(|| "webex".to_string()));
        let sender = pick_sender(&webhook_person_email, &webhook_person_id);
        let mut metadata = build_webhook_metadata(
            resource,
            event,
            membership_id.as_ref(),
            webhook_room.as_ref(),
            webhook_person_email.as_ref(),
            webhook_person_id.as_ref(),
            None,
            None,
            cfg.default_locale.as_ref(),
            Some(200),
        );
        let idempotency_key = user_entered_idempotency_key(
            "webex",
            None,
            webhook_room.as_deref(),
            webhook_person_id
                .as_deref()
                .or(webhook_person_email.as_deref()),
            "space_membership_created",
        );
        mark_user_entered(
            &mut metadata,
            "webex",
            "space_membership_created",
            idempotency_key,
        );
        if let Some(room_id) = &webhook_room {
            metadata.insert("room_id".to_string(), room_id.clone());
        }
        if let Some(person_id) = &webhook_person_id {
            metadata.insert("user_id".to_string(), person_id.clone());
        }
        if let Some(person_email) = &webhook_person_email {
            metadata.insert("person_email".to_string(), person_email.clone());
        }
        if let Some(membership_id) = &membership_id {
            metadata.insert("membership_id".to_string(), membership_id.clone());
        }
        let envelope = build_webhook_envelope(
            String::new(),
            session_id,
            sender,
            metadata,
            Vec::new(),
            membership_id.as_ref(),
        );
        return IngestOutcome {
            envelope: Some(envelope),
            status: 200,
            error: None,
        };
    }

    // Handle Adaptive Card button clicks (Action.Submit).
    if resource == "attachmentActions" && event == "created" {
        let action_id = data.get("id").and_then(|v| v.as_str()).unwrap_or_default();
        if action_id.is_empty() {
            let metadata = build_webhook_metadata(
                resource,
                event,
                None,
                None,
                None,
                None,
                None,
                None,
                cfg.default_locale.as_ref(),
                Some(400),
            );
            let envelope = build_webhook_envelope(
                String::new(),
                "webex".into(),
                None,
                metadata,
                Vec::new(),
                None,
            );
            return IngestOutcome {
                envelope: Some(envelope),
                status: 400,
                error: Some("attachmentActions missing action id".into()),
            };
        }
        let api_base = cfg
            .api_base_url
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(DEFAULT_API_BASE)
            .trim_end_matches('/')
            .to_string();
        let session_id = webhook_room
            .clone()
            .unwrap_or_else(|| action_id.to_string());
        let sender = pick_sender(&webhook_person_email, &webhook_person_id);

        // Fetch action details to get user inputs.
        let inputs = match get_secret_string(DEFAULT_TOKEN_KEY) {
            Ok(token) => match fetch_action_details(action_id, &api_base, &token) {
                Ok(details) => details,
                Err(err) => {
                    let detail = redact::error_message(&err);
                    telemetry::emit(
                        Level::Warn,
                        PROVIDER_TYPE,
                        "webex fetch action details failed",
                        &[
                            Field {
                                key: field::EVENT_KIND,
                                value: event::DOWNSTREAM_ERROR,
                            },
                            Field {
                                key: field::MESSAGE_ID,
                                value: action_id,
                            },
                            Field {
                                key: field::ERROR,
                                value: &detail,
                            },
                        ],
                    );
                    json!({})
                }
            },
            Err(_) => json!({}),
        };

        let action_id_str = action_id.to_string();
        let route_to_card = inputs
            .get("routeToCardId")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let card_id = inputs
            .get("cardId")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let action_text = if !route_to_card.is_empty() {
            format!("[card:{route_to_card}]")
        } else {
            format!("[action:{card_id}]")
        };

        let mut metadata = build_webhook_metadata(
            resource,
            event,
            Some(&action_id_str),
            webhook_room.as_ref(),
            webhook_person_email.as_ref(),
            webhook_person_id.as_ref(),
            None,
            None,
            cfg.default_locale.as_ref(),
            Some(200),
        );
        if !route_to_card.is_empty() {
            metadata.insert("routeToCardId".into(), route_to_card);
        }
        if !card_id.is_empty() {
            metadata.insert("cardId".into(), card_id);
        }
        // Forward ALL input fields to metadata for MCP routing
        if let Some(obj) = inputs.as_object() {
            for (k, v) in obj {
                let s = match v {
                    Value::String(s) => s.clone(),
                    _ => v.to_string(),
                };
                metadata.insert(k.clone(), s);
            }
        }
        metadata.insert(
            "webex.actionInputs".into(),
            serde_json::to_string(&inputs).unwrap_or_default(),
        );

        let envelope = build_webhook_envelope(
            action_text,
            session_id,
            sender,
            metadata,
            Vec::new(),
            Some(&action_id_str),
        );
        return IngestOutcome {
            envelope: Some(envelope),
            status: 200,
            error: None,
        };
    }

    if resource == "messages"
        && event == "created"
        && let Some(message_id) = message_id.clone()
    {
        let api_base = cfg
            .api_base_url
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(DEFAULT_API_BASE)
            .trim_end_matches('/')
            .to_string();
        match get_secret_string(DEFAULT_TOKEN_KEY) {
            Ok(token) => match fetch_message_details(&message_id, &api_base, &token) {
                Ok(details) => {
                    let envelope = envelope_from_details(
                        &details,
                        &message_id,
                        webhook_room.as_ref(),
                        webhook_person_email.as_ref(),
                        webhook_person_id.as_ref(),
                        resource,
                        event,
                        cfg.default_locale.as_ref(),
                    );
                    return IngestOutcome {
                        envelope: Some(envelope),
                        status: 200,
                        error: None,
                    };
                }
                Err(err) => {
                    let detail = redact::error_message(&err);
                    telemetry::emit(
                        Level::Warn,
                        PROVIDER_TYPE,
                        "webex ingest fetch error",
                        &[
                            Field {
                                key: field::EVENT_KIND,
                                value: event::DOWNSTREAM_ERROR,
                            },
                            Field {
                                key: field::MESSAGE_ID,
                                value: &message_id,
                            },
                            Field {
                                key: field::ERROR,
                                value: &detail,
                            },
                        ],
                    );
                    let session_id = webhook_room.clone().unwrap_or_else(|| message_id.clone());
                    let sender = pick_sender(&webhook_person_email, &webhook_person_id);
                    let metadata = build_webhook_metadata(
                        resource,
                        event,
                        Some(&message_id),
                        webhook_room.as_ref(),
                        webhook_person_email.as_ref(),
                        webhook_person_id.as_ref(),
                        Some(&err),
                        None,
                        cfg.default_locale.as_ref(),
                        Some(502),
                    );
                    let envelope = build_webhook_envelope(
                        "".to_string(),
                        session_id,
                        sender,
                        metadata,
                        Vec::new(),
                        Some(&message_id),
                    );
                    return IngestOutcome {
                        envelope: Some(envelope),
                        status: 502,
                        error: Some(err),
                    };
                }
            },
            Err(err) => {
                let session_id = webhook_room.clone().unwrap_or_else(|| message_id.clone());
                let sender = pick_sender(&webhook_person_email, &webhook_person_id);
                let metadata = build_webhook_metadata(
                    resource,
                    event,
                    Some(&message_id),
                    webhook_room.as_ref(),
                    webhook_person_email.as_ref(),
                    webhook_person_id.as_ref(),
                    Some(&err),
                    None,
                    cfg.default_locale.as_ref(),
                    Some(500),
                );
                let envelope = build_webhook_envelope(
                    "".to_string(),
                    session_id,
                    sender,
                    metadata,
                    Vec::new(),
                    Some(&message_id),
                );
                return IngestOutcome {
                    envelope: Some(envelope),
                    status: 500,
                    error: Some(err),
                };
            }
        }
    }

    let text = body
        .get("text")
        .or_else(|| body.get("markdown"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let session_id = webhook_room
        .clone()
        .unwrap_or_else(|| message_id.clone().unwrap_or_else(|| "webex".to_string()));
    let sender = pick_sender(&webhook_person_email, &webhook_person_id);
    let metadata = build_webhook_metadata(
        resource,
        event,
        message_id.as_ref(),
        webhook_room.as_ref(),
        webhook_person_email.as_ref(),
        webhook_person_id.as_ref(),
        None,
        None,
        cfg.default_locale.as_ref(),
        Some(200),
    );
    let envelope = build_webhook_envelope(
        text,
        session_id,
        sender,
        metadata,
        Vec::new(),
        message_id.as_ref(),
    );
    IngestOutcome {
        envelope: Some(envelope),
        status: 200,
        error: None,
    }
}

fn is_webex_bot_email(value: &str) -> bool {
    value.to_ascii_lowercase().ends_with("@webex.bot")
}

#[cfg(test)]
mod signature_tests {
    use super::*;
    use greentic_types::messaging::universal_dto::Header;

    #[test]
    fn verifies_x_spark_signature_hmac_sha1() -> Result<(), String> {
        let body = br#"{"resource":"messages","event":"created"}"#;
        let signature = hmac_sha1_hex(b"secret", body).ok_or("hmac")?;
        let headers = vec![Header {
            name: "X-Spark-Signature".to_string(),
            value: signature,
        }];

        assert!(verify_webex_signature(&headers, body, "secret"));
        assert!(!verify_webex_signature(&headers, body, "wrong"));
        Ok(())
    }

    #[test]
    fn ignores_bot_authored_message_webhooks() {
        let cfg = ProviderConfig {
            enabled: true,
            public_base_url: "https://example.com".to_string(),
            default_room_id: None,
            default_to_person_email: None,
            api_base_url: Some(DEFAULT_API_BASE.to_string()),
            bot_token: None,
            webhook_secret: None,
            default_locale: None,
        };
        let outcome = handle_webhook_event(
            &json!({
                "resource": "messages",
                "event": "created",
                "data": {
                    "id": "message-1",
                    "roomId": "room-1",
                    "personId": "bot-person",
                    "personEmail": "greentic_ci@webex.bot"
                }
            }),
            &cfg,
        );

        assert_eq!(outcome.status, 200);
        assert!(outcome.error.is_none());
        assert!(outcome.envelope.is_none());
    }

    #[test]
    fn membership_created_returns_user_entered_envelope_without_message_lookup() {
        let cfg = ProviderConfig {
            enabled: true,
            public_base_url: "https://example.com".to_string(),
            default_room_id: None,
            default_to_person_email: None,
            api_base_url: Some(DEFAULT_API_BASE.to_string()),
            bot_token: None,
            webhook_secret: None,
            default_locale: None,
        };
        let outcome = handle_webhook_event(
            &json!({
                "resource": "memberships",
                "event": "created",
                "data": {
                    "id": "membership-1",
                    "roomId": "room-1",
                    "personId": "person-1",
                    "personEmail": "ada@example.com"
                }
            }),
            &cfg,
        );

        assert_eq!(outcome.status, 200);
        assert!(outcome.error.is_none());
        let envelope = outcome.envelope.expect("envelope");
        assert_eq!(envelope.session_id, "room-1");
        assert_eq!(envelope.text.as_deref(), Some(""));
        assert_eq!(
            envelope.metadata.get("event_type").map(String::as_str),
            Some("channel.user.entered")
        );
        assert_eq!(
            envelope.metadata.get("autoStart").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            envelope.metadata.get("provider").map(String::as_str),
            Some("webex")
        );
        assert_eq!(
            envelope.metadata.get("reason").map(String::as_str),
            Some("space_membership_created")
        );
        assert_eq!(
            envelope.metadata.get("idempotency_key").map(String::as_str),
            Some("lifecycle.user_entered:webex:_:room-1:person-1:space_membership_created")
        );
        assert_eq!(
            envelope.metadata.get("room_id").map(String::as_str),
            Some("room-1")
        );
        assert_eq!(
            envelope.metadata.get("user_id").map(String::as_str),
            Some("person-1")
        );
    }

    #[test]
    fn webex_bot_email_detection_is_case_insensitive() {
        assert!(is_webex_bot_email("Greentic_CI@WEBEX.BOT"));
        assert!(!is_webex_bot_email("maarten@greentic.ai"));
    }
}
