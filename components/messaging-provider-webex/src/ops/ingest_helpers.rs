//! Low-level helpers for the ingest path.
//!
//! These functions handle the HTTP calls Webex webhooks do *not* carry inline
//! (fetching message bodies and attachment action inputs), plus the glue that
//! turns Webex API responses into [`ChannelMessageEnvelope`] values.

use greentic_types::{
    Actor, Attachment, ChannelMessageEnvelope, Destination, EnvId, MessageMetadata, TenantCtx,
    TenantId,
};
use provider_common::attachment_fetch::{FetchRef, PendingAttachment, apply_fetch_refs};
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
    /// Interactive cards (`attachments[]`); never downloadable files.
    pub(super) attachments: Vec<Attachment>,
    /// Uploaded files (`files[]`) as fetch references, applied to the
    /// envelope with `apply_fetch_refs`.
    pub(super) pending: Vec<PendingAttachment>,
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
    parse_message_details_body(resp.body.as_deref().unwrap_or_default())
}

fn parse_message_details_body(body: &[u8]) -> Result<MessageDetails, String> {
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
    let attachments = convert_webex_attachments(&data);
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
        attachments,
        pending: convert_webex_files(&data),
    })
}

/// Webex file URLs are trusted only on exactly `webexapis.com` (https, no
/// userinfo, no port) so a crafted message cannot make the host send the bot
/// token elsewhere.
fn is_webex_file_url(url: &str) -> bool {
    url.strip_prefix("https://")
        .and_then(|rest| rest.split(['/', '?', '#']).next())
        .is_some_and(|host| host == "webexapis.com")
}

/// `files[]` of a Webex message as fetch references. The message JSON carries
/// no name or MIME type (they are response headers of the download), and a
/// provider never downloads, so the type is declared unknown
/// (`application/octet-stream`) and the host sniffs the bytes. Only the NAME of
/// the bot token travels.
pub(super) fn convert_webex_files(data: &Value) -> Vec<PendingAttachment> {
    data.get("files")
        .and_then(Value::as_array)
        .map(|files| {
            files
                .iter()
                .filter_map(Value::as_str)
                .filter(|url| is_webex_file_url(url))
                .map(|url| PendingAttachment {
                    mime_type: "application/octet-stream".to_string(),
                    name: None,
                    size_bytes: None,
                    fetch: FetchRef::Bearer {
                        url: url.to_string(),
                        secret_key: crate::DEFAULT_TOKEN_KEY.to_string(),
                    },
                    inline_base64: None,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn convert_webex_attachments(data: &Value) -> Vec<Attachment> {
    data.get("attachments")
        .and_then(Value::as_array)
        .map(|array| array.iter().filter_map(build_webex_attachment).collect())
        .unwrap_or_default()
}

fn build_webex_attachment(value: &Value) -> Option<Attachment> {
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
        .map(|s| s.to_string());
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
        url,
        content: None,
        name,
        size_bytes,
    })
}

/// Envelope for a fetched `messages/created` message: cards stay in
/// `attachments` with a `{"kind":"none"}` fetch entry, kept files are appended
/// with their bearer refs. `webex.hasAttachments` / `attachmentTypes` describe
/// every attachment on the envelope (cards and kept files; a file's type is
/// `application/octet-stream`, unknown at the edge).
#[allow(clippy::too_many_arguments)]
pub(super) fn envelope_from_details(
    details: &MessageDetails,
    message_id: &String,
    webhook_room: Option<&String>,
    webhook_person_email: Option<&String>,
    webhook_person_id: Option<&String>,
    resource: &str,
    event: &str,
    default_locale: Option<&String>,
) -> ChannelMessageEnvelope {
    let session_id = details
        .room_id
        .clone()
        .or_else(|| webhook_room.cloned())
        .unwrap_or_else(|| message_id.clone());
    let sender = pick_sender(&details.person_email, &details.person_id)
        .or_else(|| pick_sender(&webhook_person_email.cloned(), &webhook_person_id.cloned()));
    let text = details
        .markdown
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| details.text.clone())
        .unwrap_or_default();
    let mut envelope = build_webhook_envelope(
        text,
        session_id,
        sender,
        MessageMetadata::new(),
        details.attachments.clone(),
        Some(message_id),
    );
    apply_fetch_refs(&mut envelope, details.pending.clone());
    let attachment_types = if envelope.attachments.is_empty() {
        None
    } else {
        Some(
            envelope
                .attachments
                .iter()
                .map(|a| a.mime_type.clone())
                .collect::<Vec<_>>()
                .join(","),
        )
    };
    let mut metadata = build_webhook_metadata(
        resource,
        event,
        Some(message_id),
        details.room_id.as_ref().or(webhook_room),
        details.person_email.as_ref().or(webhook_person_email),
        details.person_id.as_ref().or(webhook_person_id),
        None,
        attachment_types,
        default_locale,
        Some(200),
    );
    if let Some(dropped) = envelope.metadata.get("attachments_dropped") {
        metadata.insert("attachments_dropped".to_string(), dropped.clone());
    }
    envelope.metadata = metadata;
    envelope
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
    fn attachment_conversion_prefers_content_url_and_never_invents_one() {
        let data = json!({
            "attachments": [
                {"contentType": "image/png", "contentUrl": "https://cdn.example/a.png", "name": "a"},
                {"contentType": "application/json", "content": {"url": "https://cdn.example/card.json"}, "displayName": "card", "sizeBytes": 42},
                {"contentType": "text/plain", "size": 7}
            ]
        });

        let attachments = convert_webex_attachments(&data);

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
        assert_eq!(attachments[2].url, None, "no invented placeholder URL");
        assert_eq!(attachments[2].size_bytes, Some(7));
    }

    #[test]
    fn files_array_becomes_bearer_refs_on_webexapis() {
        let data = json!({"files":[
            "https://webexapis.com/v1/contents/abc123",
            "https://evil.example/x"
        ]});
        let pending = convert_webex_files(&data);
        assert_eq!(pending.len(), 1, "only webexapis.com is trusted");
        assert_eq!(
            pending[0].fetch,
            FetchRef::Bearer {
                url: "https://webexapis.com/v1/contents/abc123".into(),
                secret_key: "WEBEX_BOT_TOKEN".into()
            }
        );
        assert_eq!(pending[0].mime_type, "application/octet-stream");
        assert_eq!(pending[0].name, None);
    }

    #[test]
    fn lookalike_and_unsafe_file_hosts_are_rejected() {
        for url in [
            "https://webexapis.com.evil.example/v1/contents/a",
            "https://evil.example/webexapis.com",
            "https://webexapis.com@evil.example/v1/contents/a",
            "https://evil.example@webexapis.com/v1/contents/a",
            "https://webexapis.com:8443/v1/contents/a",
            "http://webexapis.com/v1/contents/a",
            "https://api.webexapis.com/v1/contents/a",
            "https://WEBEXAPIS.COM.evil.example/",
            "https://webexapis.com\\@evil.example/",
            "webexapis.com/v1/contents/a",
        ] {
            assert!(
                convert_webex_files(&json!({ "files": [url] })).is_empty(),
                "{url} must be rejected"
            );
        }
    }

    #[test]
    fn card_attachments_are_not_files() {
        let data = json!({"attachments":[{"contentType":"application/vnd.microsoft.card.adaptive","content":{}}]});
        assert!(convert_webex_files(&data).is_empty());
    }

    #[test]
    fn message_details_carry_files_beside_cards() {
        let details = parse_message_details_body(br#"{"text":"x","files":["https://webexapis.com/v1/contents/f1"],
                "attachments":[{"contentType":"application/vnd.microsoft.card.adaptive","content":{}}]}"#,
        )
        .expect("details");
        assert_eq!(details.attachments.len(), 1);
        assert_eq!(details.pending.len(), 1);
    }

    fn details_of(body: &str) -> MessageDetails {
        parse_message_details_body(body.as_bytes()).expect("details")
    }

    const CARD: &str = r#"{"contentType":"application/vnd.microsoft.card.adaptive","content":{"type":"AdaptiveCard"}}"#;

    fn envelope_of(details: &MessageDetails) -> ChannelMessageEnvelope {
        let id = "msg-1".to_string();
        envelope_from_details(details, &id, None, None, None, "messages", "created", None)
    }

    #[test]
    fn message_with_cards_and_files_keeps_attachments_and_refs_aligned() {
        let details = details_of(&format!(
            r#"{{"text":"x","roomId":"r","attachments":[{CARD},{CARD}],
                "files":["https://webexapis.com/v1/contents/f1","https://webexapis.com/v1/contents/f2"]}}"#
        ));
        let env = envelope_of(&details);
        assert_eq!(env.attachments.len(), 4);
        let refs = env.extensions["attachment_fetch"].as_array().expect("refs");
        assert_eq!(refs.len(), 4);
        assert_eq!(refs[0], json!({"kind":"none"}));
        assert_eq!(refs[1], json!({"kind":"none"}));
        assert_eq!(refs[2]["url"], "https://webexapis.com/v1/contents/f1");
        assert_eq!(refs[3]["url"], "https://webexapis.com/v1/contents/f2");
        assert_eq!(
            env.attachments[0].mime_type,
            "application/vnd.microsoft.card.adaptive"
        );
        assert!(env.attachments[2].url.is_none());
        assert_eq!(env.attachments[3].mime_type, "application/octet-stream");
        assert_eq!(
            env.metadata.get("webex.hasAttachments").map(String::as_str),
            Some("true")
        );
    }

    #[test]
    fn three_cards_and_five_files_keep_all_files_cards_do_not_use_the_cap() {
        let files: Vec<String> = (0..7)
            .map(|i| format!("\"https://webexapis.com/v1/contents/f{i}\""))
            .collect();
        let details = details_of(&format!(
            r#"{{"text":"x","attachments":[{CARD},{CARD},{CARD}],"files":[{}]}}"#,
            files[..5].join(",")
        ));
        let env = envelope_of(&details);
        assert_eq!(env.attachments.len(), 8);
        assert!(!env.metadata.contains_key("attachments_dropped"));

        let over = details_of(&format!(r#"{{"text":"x","files":[{}]}}"#, files.join(",")));
        let env = envelope_of(&over);
        assert_eq!(env.attachments.len(), 5, "the cap is 5 files");
        assert_eq!(
            env.metadata.get("attachments_dropped").map(String::as_str),
            Some("2")
        );
    }

    #[test]
    fn the_same_file_url_twice_is_one_reference() {
        let details = details_of(
            r#"{"text":"x","files":["https://webexapis.com/v1/contents/f1","https://webexapis.com/v1/contents/f1"]}"#,
        );
        assert_eq!(envelope_of(&details).attachments.len(), 1);
    }

    #[test]
    fn a_10kb_file_url_is_refused_and_counted() {
        let url = format!(
            "https://webexapis.com/v1/contents/{}",
            "a".repeat(10 * 1024)
        );
        let details = details_of(&format!(r#"{{"text":"x","files":["{url}"]}}"#));
        let env = envelope_of(&details);
        assert!(env.attachments.is_empty());
        assert_eq!(
            env.metadata.get("attachments_dropped").map(String::as_str),
            Some("1")
        );
        assert_eq!(
            env.metadata.get("webex.hasAttachments").map(String::as_str),
            Some("false")
        );
    }

    #[test]
    fn a_files_only_message_reports_has_attachments() {
        let details =
            details_of(r#"{"text":"x","files":["https://webexapis.com/v1/contents/f1"]}"#);
        let env = envelope_of(&details);
        assert_eq!(
            env.metadata.get("webex.hasAttachments").map(String::as_str),
            Some("true")
        );
        assert!(env.extensions["attachment_fetch"][0]["kind"] == "bearer");
    }

    #[test]
    fn a_message_without_files_or_cards_is_unchanged() {
        let env = envelope_of(&details_of(r#"{"text":"x"}"#));
        assert!(env.attachments.is_empty());
        assert!(env.extensions.is_empty());
        assert_eq!(
            env.metadata.get("webex.hasAttachments").map(String::as_str),
            Some("false")
        );
    }

    #[test]
    fn attachment_conversion_handles_missing_array() {
        assert!(convert_webex_attachments(&json!({})).is_empty());
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
        let err =
            parse_message_details_body(b"{not-json").expect_err("invalid message json should fail");
        assert!(err.contains("invalid message JSON"));
    }
}
