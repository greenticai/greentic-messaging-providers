//! Low-level helpers for the ingest path.
//!
//! These functions handle the HTTP calls Webex webhooks do *not* carry inline
//! (fetching message bodies and attachment action inputs), plus the glue that
//! turns Webex API responses into [`ChannelMessageEnvelope`] values.

use greentic_types::{
    Actor, Attachment, ChannelMessageEnvelope, Destination, EnvId, MessageMetadata, TenantCtx,
    TenantId,
};
use provider_common::redact;
use provider_common::telemetry::{self, Field, Level, event, field};
use serde_json::{Value, json};

use super::format_webex_error;
use crate::PROVIDER_TYPE;
use crate::bindings::greentic::http::http_client as client;

/// Details extracted from `GET /messages/{id}`.
#[derive(Debug)]
pub(super) struct MessageDetails {
    pub(super) markdown: Option<String>,
    pub(super) text: Option<String>,
    pub(super) room_id: Option<String>,
    pub(super) person_email: Option<String>,
    pub(super) person_id: Option<String>,
    /// `direct` (1:1) or `group`, as reported by the fetched message object.
    pub(super) room_type: Option<String>,
    pub(super) attachments: Vec<Attachment>,
}

/// Fetch Webex attachment action details to retrieve user inputs.
pub(super) fn fetch_action_details(
    action_id: &str,
    api_base: &str,
    token: &str,
) -> Result<Value, String> {
    let url = format!("{api_base}/attachment/actions/{action_id}");
    telemetry::emit(
        Level::Debug,
        PROVIDER_TYPE,
        "webex ingest fetching action",
        &[
            Field {
                key: field::EVENT_KIND,
                value: event::DOWNSTREAM_CALL,
            },
            Field {
                key: field::HTTP_METHOD,
                value: "GET",
            },
            Field {
                key: field::HTTP_HOST,
                value: api_base,
            },
            Field {
                key: field::MESSAGE_ID,
                value: action_id,
            },
        ],
    );
    let request = client::Request {
        method: "GET".to_string(),
        url: url.clone(),
        headers: vec![("Authorization".into(), format!("Bearer {token}"))],
        body: None,
    };
    let resp = client::send(&request, None, None).map_err(|err| {
        let detail = redact::error_message(&err.message);
        telemetry::emit(
            Level::Error,
            PROVIDER_TYPE,
            "webex ingest action transport error",
            &[
                Field {
                    key: field::EVENT_KIND,
                    value: event::DOWNSTREAM_ERROR,
                },
                Field {
                    key: field::HTTP_HOST,
                    value: api_base,
                },
                Field {
                    key: field::ERROR,
                    value: &detail,
                },
            ],
        );
        format!("transport error: {detail}")
    })?;
    let status_str = resp.status.to_string();
    telemetry::emit(
        Level::Trace,
        PROVIDER_TYPE,
        "webex ingest action response",
        &[
            Field {
                key: field::MESSAGE_ID,
                value: action_id,
            },
            Field {
                key: field::HTTP_STATUS,
                value: &status_str,
            },
        ],
    );
    if resp.status < 200 || resp.status >= 300 {
        let body = resp.body.unwrap_or_default();
        let body_text = String::from_utf8_lossy(&body);
        telemetry::downstream_error(PROVIDER_TYPE, api_base, resp.status, &body_text);
        return Err(format_webex_error(resp.status, &body));
    }
    parse_action_inputs(resp.body.as_deref().unwrap_or_default())
}

fn parse_action_inputs(body: &[u8]) -> Result<Value, String> {
    let parsed: Value = serde_json::from_slice(body).map_err(|err| {
        let detail = redact::error_message(&err.to_string());
        telemetry::emit(
            Level::Error,
            PROVIDER_TYPE,
            "webex ingest action json parse failed",
            &[Field {
                key: field::ERROR,
                value: &detail,
            }],
        );
        format!("json parse error: {detail}")
    })?;
    Ok(parsed.get("inputs").cloned().unwrap_or(json!({})))
}

pub(super) fn fetch_message_details(
    message_id: &str,
    api_base: &str,
    token: &str,
) -> Result<MessageDetails, String> {
    let url = format!("{api_base}/messages/{message_id}");
    telemetry::emit(
        Level::Debug,
        PROVIDER_TYPE,
        "webex ingest fetching message",
        &[
            Field {
                key: field::EVENT_KIND,
                value: event::DOWNSTREAM_CALL,
            },
            Field {
                key: field::HTTP_METHOD,
                value: "GET",
            },
            Field {
                key: field::HTTP_HOST,
                value: api_base,
            },
            Field {
                key: field::MESSAGE_ID,
                value: message_id,
            },
        ],
    );
    let request = client::Request {
        method: "GET".to_string(),
        url: url.clone(),
        headers: vec![("Authorization".into(), format!("Bearer {token}"))],
        body: None,
    };
    let resp = client::send(&request, None, None).map_err(|err| {
        let detail = redact::error_message(&err.message);
        telemetry::emit(
            Level::Error,
            PROVIDER_TYPE,
            "webex ingest message transport error",
            &[
                Field {
                    key: field::EVENT_KIND,
                    value: event::DOWNSTREAM_ERROR,
                },
                Field {
                    key: field::HTTP_HOST,
                    value: api_base,
                },
                Field {
                    key: field::ERROR,
                    value: &detail,
                },
            ],
        );
        format!("transport error: {detail}")
    })?;
    let status_str = resp.status.to_string();
    telemetry::emit(
        Level::Trace,
        PROVIDER_TYPE,
        "webex ingest message response",
        &[
            Field {
                key: field::MESSAGE_ID,
                value: message_id,
            },
            Field {
                key: field::HTTP_STATUS,
                value: &status_str,
            },
        ],
    );
    if resp.status < 200 || resp.status >= 300 {
        let body = resp.body.unwrap_or_default();
        let body_text = String::from_utf8_lossy(&body);
        telemetry::downstream_error(PROVIDER_TYPE, api_base, resp.status, &body_text);
        return Err(format_webex_error(resp.status, &body));
    }
    parse_message_details_body(message_id, resp.body.as_deref().unwrap_or_default())
}

fn parse_message_details_body(message_id: &str, body: &[u8]) -> Result<MessageDetails, String> {
    let message_json: Value = serde_json::from_slice(body).map_err(|err| {
        let detail = redact::error_message(&err.to_string());
        telemetry::emit(
            Level::Error,
            PROVIDER_TYPE,
            "webex ingest message json parse failed",
            &[Field {
                key: field::ERROR,
                value: &detail,
            }],
        );
        format!("invalid message JSON: {detail}")
    })?;
    let data = message_json
        .get("result")
        .cloned()
        .unwrap_or_else(|| message_json.clone());
    let attachments = convert_webex_attachments(message_id, &data);
    Ok(MessageDetails {
        markdown: data
            .get("markdown")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        text: data
            .get("text")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        room_id: data
            .get("roomId")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        person_email: data
            .get("personEmail")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        person_id: data
            .get("personId")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        room_type: data
            .get("roomType")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        attachments,
    })
}

fn convert_webex_attachments(message_id: &str, data: &Value) -> Vec<Attachment> {
    data.get("attachments")
        .and_then(Value::as_array)
        .map(|array| {
            array
                .iter()
                .enumerate()
                .filter_map(|(idx, attachment)| build_webex_attachment(message_id, idx, attachment))
                .collect()
        })
        .unwrap_or_default()
}

fn build_webex_attachment(message_id: &str, idx: usize, value: &Value) -> Option<Attachment> {
    let mime_type = value
        .get("contentType")
        .and_then(|v| v.as_str())
        .unwrap_or("application/octet-stream")
        .to_string();
    let url = value
        .get("contentUrl")
        .and_then(|v| v.as_str())
        .or_else(|| {
            value
                .get("content")
                .and_then(|content| content.get("url"))
                .and_then(|v| v.as_str())
        })
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("webex:{message_id}:attachment:{idx}"));
    let name = value
        .get("name")
        .or_else(|| value.get("displayName"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let size_bytes = value
        .get("size")
        .and_then(|v| v.as_u64())
        .or_else(|| value.get("sizeBytes").and_then(|v| v.as_u64()));
    Some(Attachment {
        mime_type,
        url: Some(url),
        content: None,
        name,
        size_bytes,
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_webhook_metadata(
    resource: &str,
    event: &str,
    message_id: Option<&String>,
    room_id: Option<&String>,
    person_email: Option<&String>,
    person_id: Option<&String>,
    error: Option<&String>,
    attachment_types: Option<String>,
    default_locale: Option<&String>,
    status: Option<u16>,
) -> MessageMetadata {
    let mut metadata = MessageMetadata::new();
    metadata.insert("webex.resource".to_string(), resource.to_string());
    metadata.insert("webex.event".to_string(), event.to_string());
    if let Some(msg) = message_id {
        metadata.insert("webex.messageId".to_string(), msg.clone());
    }
    if let Some(room) = room_id {
        metadata.insert("webex.roomId".to_string(), room.clone());
    }
    if let Some(email) = person_email {
        metadata.insert("webex.personEmail".to_string(), email.clone());
    }
    if let Some(id) = person_id {
        metadata.insert("webex.personId".to_string(), id.clone());
    }
    if let Some(err) = error {
        metadata.insert("webex.ingestError".to_string(), err.clone());
    }
    if let Some(status) = status {
        metadata.insert("webex.fetchStatus".to_string(), status.to_string());
    }
    metadata.insert(
        "webex.hasAttachments".to_string(),
        attachment_types.is_some().to_string(),
    );
    if let Some(types) = attachment_types {
        metadata.insert("webex.attachmentTypes".to_string(), types);
    }
    // Webex doesn't include locale in webhooks; use provider config default.
    if let Some(locale) = default_locale
        && !locale.is_empty()
    {
        metadata.insert("locale".to_string(), locale.clone());
    }
    metadata
}

pub(super) fn build_webhook_envelope(
    text: String,
    session_id: String,
    from: Option<Actor>,
    metadata: MessageMetadata,
    attachments: Vec<Attachment>,
    message_id: Option<&String>,
) -> ChannelMessageEnvelope {
    let env = EnvId::try_from("default").expect("env id");
    let tenant = TenantId::try_from("default").expect("tenant id");
    let destinations = if !session_id.is_empty() {
        vec![Destination {
            id: session_id.clone(),
            kind: None,
        }]
    } else {
        Vec::new()
    };
    ChannelMessageEnvelope {
        id: message_id
            .map(|id| format!("webex-{id}"))
            .unwrap_or_else(|| format!("webex-ingress-{session_id}")),
        tenant: TenantCtx::new(env.clone(), tenant.clone()),
        channel: "webex".to_string(),
        session_id: session_id.clone(),
        reply_scope: None,
        from,
        to: destinations,
        correlation_id: None,
        text: Some(text),
        attachments,
        metadata,
        extensions: Default::default(),
    }
}

pub(super) fn pick_sender(
    person_email: &Option<String>,
    person_id: &Option<String>,
) -> Option<Actor> {
    if let Some(email) = person_email {
        return Some(Actor {
            id: email.clone(),
            kind: Some("person".into()),
        });
    }
    if let Some(id) = person_id {
        return Some(Actor {
            id: id.clone(),
            kind: Some("person".into()),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn build_metadata_records_fetch_failures_and_locale() {
        let message_id = "msg-1".to_string();
        let room_id = "room-1".to_string();
        let error = "not found".to_string();
        let locale = "fr".to_string();

        let metadata = build_webhook_metadata(
            "messages",
            "created",
            Some(&message_id),
            Some(&room_id),
            None,
            None,
            Some(&error),
            Some("application/vnd.microsoft.card.adaptive".to_string()),
            Some(&locale),
            Some(404),
        );

        assert_eq!(
            metadata.get("webex.messageId").map(String::as_str),
            Some("msg-1")
        );
        assert_eq!(
            metadata.get("webex.fetchStatus").map(String::as_str),
            Some("404")
        );
        assert_eq!(
            metadata.get("webex.ingestError").map(String::as_str),
            Some("not found")
        );
        assert_eq!(
            metadata.get("webex.hasAttachments").map(String::as_str),
            Some("true")
        );
        assert_eq!(metadata.get("locale").map(String::as_str), Some("fr"));
    }

    #[test]
    fn build_metadata_omits_optional_empty_fields() {
        let metadata = build_webhook_metadata(
            "attachmentActions",
            "created",
            None,
            None,
            None,
            None,
            None,
            None,
            Some(&"".to_string()),
            None,
        );

        assert_eq!(
            metadata.get("webex.resource").map(String::as_str),
            Some("attachmentActions")
        );
        assert_eq!(
            metadata.get("webex.hasAttachments").map(String::as_str),
            Some("false")
        );
        assert!(!metadata.contains_key("locale"));
        assert!(!metadata.contains_key("webex.fetchStatus"));
    }

    #[test]
    fn webhook_envelope_uses_message_id_when_available() {
        let message_id = "abc".to_string();
        let envelope = build_webhook_envelope(
            "hello".to_string(),
            "room-1".to_string(),
            pick_sender(&Some("person@example.com".to_string()), &None),
            MessageMetadata::new(),
            Vec::new(),
            Some(&message_id),
        );

        assert_eq!(envelope.id, "webex-abc");
        assert_eq!(envelope.session_id, "room-1");
        assert_eq!(
            envelope.from.as_ref().map(|actor| actor.id.as_str()),
            Some("person@example.com")
        );
        assert_eq!(envelope.to[0].id, "room-1");
    }

    #[test]
    fn webhook_envelope_falls_back_without_message_or_destination() {
        let envelope = build_webhook_envelope(
            "hello".to_string(),
            "".to_string(),
            pick_sender(&None, &Some("person-id".to_string())),
            MessageMetadata::new(),
            Vec::new(),
            None,
        );

        assert_eq!(envelope.id, "webex-ingress-");
        assert!(envelope.to.is_empty());
        assert_eq!(
            envelope.from.as_ref().map(|actor| actor.id.as_str()),
            Some("person-id")
        );
    }

    #[test]
    fn attachment_conversion_prefers_content_url_but_falls_back_to_stable_id() {
        let data = json!({
            "attachments": [
                {"contentType": "image/png", "contentUrl": "https://cdn.example/a.png", "name": "a"},
                {"contentType": "application/json", "content": {"url": "https://cdn.example/card.json"}, "displayName": "card", "sizeBytes": 42},
                {"contentType": "text/plain", "size": 7}
            ]
        });

        let attachments = convert_webex_attachments("msg-1", &data);

        assert_eq!(attachments.len(), 3);
        assert_eq!(
            attachments[0].url.as_deref(),
            Some("https://cdn.example/a.png")
        );
        assert_eq!(
            attachments[1].url.as_deref(),
            Some("https://cdn.example/card.json")
        );
        assert_eq!(attachments[1].name.as_deref(), Some("card"));
        assert_eq!(attachments[1].size_bytes, Some(42));
        assert_eq!(
            attachments[2].url.as_deref(),
            Some("webex:msg-1:attachment:2")
        );
        assert_eq!(attachments[2].size_bytes, Some(7));
    }

    #[test]
    fn attachment_conversion_handles_missing_array() {
        assert!(convert_webex_attachments("msg-1", &json!({})).is_empty());
    }

    #[test]
    fn parse_action_inputs_returns_inputs_or_empty_object() {
        let inputs = parse_action_inputs(br#"{"inputs":{"approved":true,"reason":"ok"}}"#)
            .expect("valid action inputs");
        assert_eq!(inputs["approved"], true);
        assert_eq!(inputs["reason"], "ok");

        let empty = parse_action_inputs(br#"{"id":"action-1"}"#).expect("missing inputs is empty");
        assert_eq!(empty, json!({}));
    }

    #[test]
    fn parse_action_inputs_reports_invalid_json() {
        let err = parse_action_inputs(b"{not-json").expect_err("invalid json should fail");
        assert!(err.contains("json parse error"));
    }

    #[test]
    fn parse_message_details_unwraps_result_and_attachments() {
        let details = parse_message_details_body(
            "msg-1",
            br#"{
                "result": {
                    "markdown": "**hello**",
                    "text": "hello",
                    "roomId": "room-1",
                    "personEmail": "sender@example.com",
                    "personId": "person-1",
                    "attachments": [
                        {"contentType":"image/png","contentUrl":"https://cdn.example/a.png"}
                    ]
                }
            }"#,
        )
        .expect("valid message details");

        assert_eq!(details.markdown.as_deref(), Some("**hello**"));
        assert_eq!(details.text.as_deref(), Some("hello"));
        assert_eq!(details.room_id.as_deref(), Some("room-1"));
        assert_eq!(details.person_email.as_deref(), Some("sender@example.com"));
        assert_eq!(details.person_id.as_deref(), Some("person-1"));
        assert_eq!(details.attachments.len(), 1);
    }

    #[test]
    fn parse_message_details_accepts_top_level_message() {
        let details = parse_message_details_body(
            "msg-2",
            br#"{"text":"plain","roomId":"room-2","personId":"person-2"}"#,
        )
        .expect("valid top-level message");

        assert_eq!(details.text.as_deref(), Some("plain"));
        assert_eq!(details.room_id.as_deref(), Some("room-2"));
        assert_eq!(details.person_id.as_deref(), Some("person-2"));
        assert!(details.attachments.is_empty());
    }

    #[test]
    fn parse_message_details_reports_invalid_json() {
        let err = parse_message_details_body("msg-1", b"{not-json")
            .expect_err("invalid message json should fail");
        assert!(err.contains("invalid message JSON"));
    }
}
