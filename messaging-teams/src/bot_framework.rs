use provider_common::attachment_fetch::{FetchRef, PendingAttachment, UNKNOWN_MIME};
use serde_json::{Map, Value, json};

const USER_ENTERED_EVENT_TYPE: &str = "channel.user.entered";

pub fn is_bot_framework_activity(body: &Value) -> bool {
    body.get("type").and_then(Value::as_str).is_some()
        && (body.get("serviceUrl").and_then(Value::as_str).is_some()
            || body
                .get("channelId")
                .and_then(Value::as_str)
                .is_some_and(|channel| {
                    channel.eq_ignore_ascii_case("msteams") || channel.eq_ignore_ascii_case("teams")
                }))
}

pub fn handle_bot_framework_activity(
    headers_json: &str,
    activity: &Value,
) -> Result<Value, String> {
    validate_bot_framework_auth(headers_json)?;

    let activity_type = activity
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let service_url = activity
        .get("serviceUrl")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let conversation_id = activity
        .get("conversation")
        .and_then(|value| value.get("id"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    if service_url.trim().is_empty() || conversation_id.trim().is_empty() {
        return Err(
            "validation error: Bot Framework activity missing serviceUrl or conversation.id"
                .to_string(),
        );
    }

    let submit = extract_submit(activity);
    let mut events = lifecycle_events(activity);
    if events.is_empty() {
        events.push(normalize_activity(activity, submit.as_ref()));
    }
    let follow_up = follow_up_activity(activity, submit.as_ref());
    let invoke_response = if activity_type.eq_ignore_ascii_case("invoke") {
        Some(invoke_response_card(submit.as_ref()))
    } else {
        None
    };

    Ok(json!({
        "ok": true,
        "provider": "messaging.teams",
        "kind": "bot_framework_activity",
        "activity_type": activity_type,
        "service_url": service_url,
        "conversation_id": conversation_id,
        "event": activity,
        "events": events,
        "conversation": {
            "serviceUrl": service_url,
            "id": conversation_id
        },
        "reply_activity": follow_up,
        "invoke_response": invoke_response
    }))
}

fn lifecycle_events(activity: &Value) -> Vec<Value> {
    let activity_type = activity_type(activity);
    if activity_type.eq_ignore_ascii_case("conversationUpdate") {
        return members_added_lifecycle_events(activity);
    }
    if activity_type.eq_ignore_ascii_case("installationUpdate") && installation_is_add(activity) {
        return vec![build_user_entered_event(
            activity,
            activity
                .get("from")
                .and_then(|value| value.get("id"))
                .and_then(Value::as_str),
            "app_installed",
        )];
    }
    Vec::new()
}

fn members_added_lifecycle_events(activity: &Value) -> Vec<Value> {
    let bot_id = activity
        .get("recipient")
        .and_then(|value| value.get("id"))
        .and_then(Value::as_str);
    activity
        .get("membersAdded")
        .and_then(Value::as_array)
        .map(|members| {
            members
                .iter()
                .filter_map(|member| {
                    let member_id = member.get("id").and_then(Value::as_str);
                    if member_id.is_some() && member_id == bot_id {
                        return None;
                    }
                    Some(build_user_entered_event(
                        activity,
                        member_id,
                        "members_added",
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn installation_is_add(activity: &Value) -> bool {
    activity
        .get("action")
        .or_else(|| activity.get("value").and_then(|value| value.get("action")))
        .and_then(Value::as_str)
        .is_none_or(|action| action.eq_ignore_ascii_case("add"))
}

fn build_user_entered_event(activity: &Value, user_id: Option<&str>, reason: &str) -> Value {
    let activity_id = activity
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("activity");
    let conversation_id = activity
        .get("conversation")
        .and_then(|value| value.get("id"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let tenant_id = activity
        .get("channelData")
        .and_then(|value| value.get("tenant"))
        .and_then(|value| value.get("id"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let service_url = activity
        .get("serviceUrl")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let user_id = user_id
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            activity
                .get("from")
                .and_then(|value| value.get("id"))
                .and_then(Value::as_str)
        })
        .unwrap_or_default();

    let mut metadata = Map::new();
    insert_string(&mut metadata, "provider", "teams");
    insert_string(&mut metadata, "source", "teams");
    insert_string(&mut metadata, "activity_type", activity_type(activity));
    insert_string(&mut metadata, "event_type", USER_ENTERED_EVENT_TYPE);
    insert_string(&mut metadata, "autoStart", "true");
    insert_string(&mut metadata, "reason", reason);
    insert_string(&mut metadata, "service_url", service_url);
    insert_string(&mut metadata, "conversation_id", conversation_id);
    insert_string(&mut metadata, "tenant_id", tenant_id);
    insert_string(&mut metadata, "user_id", user_id);
    insert_string(
        &mut metadata,
        "idempotency_key",
        &user_entered_idempotency_key(
            "teams",
            Some(tenant_id),
            Some(conversation_id),
            Some(user_id),
            reason,
        ),
    );

    let mut event = Map::new();
    event.insert(
        "id".to_string(),
        Value::String(format!("teams-bot:{activity_id}:{reason}:{user_id}")),
    );
    event.insert(
        "tenant".to_string(),
        json!({
            "env": "default",
            "tenant": "default",
            "tenant_id": if tenant_id.is_empty() { "default" } else { tenant_id },
            "attempt": 0
        }),
    );
    event.insert("channel".to_string(), Value::String("teams".to_string()));
    event.insert(
        "session_id".to_string(),
        Value::String(if conversation_id.is_empty() {
            "teams".to_string()
        } else {
            conversation_id.to_string()
        }),
    );
    event.insert("text".to_string(), Value::String(String::new()));
    event.insert("metadata".to_string(), Value::Object(metadata));
    if !user_id.is_empty() {
        event.insert("from".to_string(), json!({"id": user_id, "kind": "user"}));
    } else if let Some(actor) = activity_actor(activity, "from") {
        event.insert("from".to_string(), actor);
    }
    if let Some(actor) = activity_actor(activity, "recipient") {
        event.insert("to".to_string(), Value::Array(vec![actor]));
    }
    Value::Object(event)
}

fn validate_bot_framework_auth(headers_json: &str) -> Result<(), String> {
    let headers: Value = serde_json::from_str(headers_json)
        .map_err(|_| "unauthorized: invalid Bot Framework headers".to_string())?;
    let authorization = header_value(&headers, "authorization")
        .or_else(|| header_value(&headers, "Authorization"))
        .unwrap_or_default();
    let token = authorization.trim();
    if token.is_empty() {
        return Err("unauthorized: Bot Framework bearer token required".to_string());
    }
    if !token.to_ascii_lowercase().starts_with("bearer ") {
        return Err("unauthorized: Bot Framework authorization must use Bearer".to_string());
    }
    if token[7..].trim().is_empty() {
        return Err("unauthorized: Bot Framework bearer token required".to_string());
    }
    Ok(())
}

fn header_value(headers: &Value, name: &str) -> Option<String> {
    let obj = headers.as_object()?;
    obj.iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .and_then(|(_, value)| value.as_str())
        .map(ToOwned::to_owned)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SubmitPayload {
    action_id: String,
    text: String,
    raw: Value,
}

fn extract_submit(activity: &Value) -> Option<SubmitPayload> {
    let mut merged = Map::new();
    let value = activity.get("value").unwrap_or(&Value::Null);
    merge_object(&mut merged, value);

    if let Some(data) = value.get("data") {
        merge_object(&mut merged, data);
    }
    if let Some(action_text) = value.get("action").and_then(Value::as_str) {
        merged
            .entry("action_id".to_string())
            .or_insert_with(|| Value::String(action_text.to_string()));
    }
    if let Some(action) = value.get("action") {
        merge_object(&mut merged, action);
        if let Some(data) = action.get("data") {
            merge_object(&mut merged, data);
        }
        if let Some(verb) = action.get("verb").and_then(Value::as_str) {
            merged
                .entry("action_id".to_string())
                .or_insert_with(|| Value::String(verb.to_string()));
        }
    }
    if let Some(msteams) = value.get("msteams") {
        merge_object(&mut merged, msteams);
        if let Some(inner) = msteams.get("value") {
            if let Some(parsed) = parse_json_string(inner) {
                merge_object(&mut merged, &parsed);
            } else {
                merge_object(&mut merged, inner);
            }
        }
    }

    let action_id = first_string(
        &merged,
        &[
            "action_id",
            "action",
            "verb",
            "button",
            "submitAction",
            "id",
        ],
    )
    .unwrap_or_else(|| "submit".to_string());
    let text = first_string(
        &merged,
        &[
            "text",
            "inputText",
            "comment",
            "message",
            "value",
            "displayText",
        ],
    )
    .unwrap_or_default();

    if merged.is_empty() && !activity_type(activity).eq_ignore_ascii_case("invoke") {
        return None;
    }

    Some(SubmitPayload {
        action_id,
        text,
        raw: Value::Object(merged),
    })
}

fn merge_object(target: &mut Map<String, Value>, value: &Value) {
    let Some(obj) = value.as_object() else {
        return;
    };
    for (key, val) in obj {
        if key == "msteams" || key == "data" || key == "action" {
            continue;
        }
        target.entry(key.clone()).or_insert_with(|| val.clone());
    }
}

fn parse_json_string(value: &Value) -> Option<Value> {
    let text = value.as_str()?.trim();
    if text.is_empty() {
        return None;
    }
    serde_json::from_str(text).ok()
}

fn first_string(map: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        map.get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    })
}

const FILE_DOWNLOAD_INFO: &str = "application/vnd.microsoft.teams.file.download.info";

/// Inbound file references plus the number of attachments dropped here,
/// before the shared checks in `apply_to_value` (which count their own).
pub(crate) struct TeamsAttachments {
    pub(crate) pending: Vec<PendingAttachment>,
    pub(crate) skipped: usize,
}

/// Declared type from a Teams `fileType` (or the name's extension). Only a
/// hint: the host sniffs the bytes. Unknown types become `UNKNOWN_MIME`,
/// which the shared guard drops (and counts) for a `public` ref.
fn mime_for_extension(file_type: &str) -> &'static str {
    match file_type.to_ascii_lowercase().as_str() {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "txt" => "text/plain",
        "md" | "markdown" => "text/markdown",
        "csv" => "text/csv",
        "json" => "application/json",
        _ => UNKNOWN_MIME,
    }
}

/// A Teams `downloadUrl` is a pre-authenticated SharePoint/OneDrive link.
/// Only `https://<dns name>.sharepoint.com` with no userinfo and no port is
/// trusted, so a crafted activity cannot point the host at another target
/// (lookalike domains, IP literals, `localhost`). The shared guard checks the
/// rest of the url shape and its length.
fn is_teams_download_host(url: &str) -> bool {
    let Some(rest) = url
        .get(..8)
        .filter(|p| p.eq_ignore_ascii_case("https://"))
        .map(|_| &url[8..])
    else {
        return false;
    };
    let host = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    host.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        && host.ends_with(".sharepoint.com")
        && !host.starts_with('.')
        && !host.contains("..")
}

fn warn_skipped(reason: &str) {
    // A fixed reason only: never the url (a credential) or the file name.
    use provider_common::telemetry::{Field, Level, field, log};
    log(
        Level::Warn,
        "attachment dropped at the provider edge",
        &[
            Field {
                key: field::PROVIDER,
                value: "teams",
            },
            Field {
                key: "reason",
                value: reason,
            },
        ],
    );
}

fn download_info_ref(attachment: &Value) -> Result<PendingAttachment, &'static str> {
    let content = attachment
        .get("content")
        .ok_or("file without a download url")?;
    let url = content
        .get("downloadUrl")
        .and_then(Value::as_str)
        .ok_or("file without a download url")?;
    if !is_teams_download_host(url) {
        return Err("download url host not allowed");
    }
    let name = attachment.get("name").and_then(Value::as_str);
    let file_type = content
        .get("fileType")
        .and_then(Value::as_str)
        .or_else(|| name.and_then(|n| n.rsplit_once('.').map(|(_, ext)| ext)))
        .unwrap_or_default();
    Ok(PendingAttachment {
        mime_type: mime_for_extension(file_type).to_string(),
        name: name.map(str::to_string),
        size_bytes: None,
        fetch: FetchRef::Public {
            url: url.to_string(),
        },
        inline_base64: None,
    })
}

/// `activity.attachments[]` read for files. A file upload
/// (`file.download.info`) becomes a `public` fetch ref. Inline images need
/// the Bot Framework token, which the host does not hold: not served in v1,
/// skipped and counted. Cards (`application/vnd.microsoft.card.*`) and any
/// other content type are not files and are neither kept nor counted.
pub(crate) fn teams_attachments(activity: &Value) -> TeamsAttachments {
    let mut out = TeamsAttachments {
        pending: Vec::new(),
        skipped: 0,
    };
    let Some(items) = activity.get("attachments").and_then(Value::as_array) else {
        return out;
    };
    for item in items {
        let content_type = item
            .get("contentType")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let result = if content_type == FILE_DOWNLOAD_INFO {
            download_info_ref(item).map(Some)
        } else if content_type.to_ascii_lowercase().starts_with("image/") {
            Err("inline image not served")
        } else {
            Ok(None)
        };
        match result {
            Ok(Some(pending)) => out.pending.push(pending),
            Ok(None) => {}
            Err(reason) => {
                warn_skipped(reason);
                out.skipped += 1;
            }
        }
    }
    out
}

/// The file refs alone (the skipped count is in [`teams_attachments`]).
#[cfg(test)]
pub(crate) fn teams_pending_attachments(activity: &Value) -> Vec<PendingAttachment> {
    teams_attachments(activity).pending
}

/// Apply the file refs to a raw-JSON event and add the locally skipped
/// attachments to `metadata.attachments_dropped`.
fn apply_teams_attachments(event: &mut Value, activity: &Value) {
    let parsed = teams_attachments(activity);
    provider_common::attachment_fetch::apply_to_value(event, parsed.pending);
    if parsed.skipped == 0 {
        return;
    }
    let Some(metadata) = event.get_mut("metadata").and_then(Value::as_object_mut) else {
        return;
    };
    let previous = metadata
        .get("attachments_dropped")
        .and_then(Value::as_str)
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    metadata.insert(
        "attachments_dropped".to_string(),
        Value::String((previous + parsed.skipped).to_string()),
    );
}

fn normalize_activity(activity: &Value, submit: Option<&SubmitPayload>) -> Value {
    let activity_id = activity
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("activity");
    let conversation = activity.get("conversation").unwrap_or(&Value::Null);
    let conversation_id = conversation
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let tenant_id = activity
        .get("channelData")
        .and_then(|value| value.get("tenant"))
        .and_then(|value| value.get("id"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let text = submit
        .map(|payload| payload.text.as_str())
        .filter(|value| !value.is_empty())
        .or_else(|| activity.get("text").and_then(Value::as_str))
        .unwrap_or_default();

    let mut metadata = Map::new();
    insert_string(&mut metadata, "provider", "messaging.teams");
    insert_string(&mut metadata, "source", "teams");
    insert_string(&mut metadata, "activity_type", activity_type(activity));
    insert_string(
        &mut metadata,
        "service_url",
        activity
            .get("serviceUrl")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    insert_string(&mut metadata, "conversation_id", conversation_id);
    insert_string(&mut metadata, "tenant_id", tenant_id);
    if let Some(payload) = submit {
        insert_string(&mut metadata, "action_id", &payload.action_id);
        insert_string(&mut metadata, "submitted_text", &payload.text);
        metadata.insert("submit".to_string(), payload.raw.clone());
    }

    let mut event = Map::new();
    event.insert(
        "id".to_string(),
        Value::String(format!("teams-bot:{activity_id}")),
    );
    event.insert(
        "tenant".to_string(),
        json!({
            "env": "default",
            "tenant": "default",
            "tenant_id": if tenant_id.is_empty() { "default" } else { tenant_id },
            "attempt": 0
        }),
    );
    event.insert("channel".to_string(), Value::String("teams".to_string()));
    event.insert(
        "session_id".to_string(),
        Value::String(if conversation_id.is_empty() {
            "teams".to_string()
        } else {
            conversation_id.to_string()
        }),
    );
    event.insert("text".to_string(), Value::String(text.to_string()));
    event.insert(
        "provider_message_id".to_string(),
        Value::String(format!("teams-bot:{activity_id}")),
    );
    event.insert("metadata".to_string(), Value::Object(metadata));
    if let Some(actor) = activity_actor(activity, "from") {
        event.insert("from".to_string(), actor);
    }
    if let Some(actor) = activity_actor(activity, "recipient") {
        event.insert("to".to_string(), Value::Array(vec![actor]));
    }
    let mut event = Value::Object(event);
    apply_teams_attachments(&mut event, activity);
    event
}

fn activity_type(activity: &Value) -> &str {
    activity
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

fn activity_actor(activity: &Value, key: &str) -> Option<Value> {
    let actor = activity.get(key)?;
    let id = actor.get("id").and_then(Value::as_str).unwrap_or_default();
    if id.trim().is_empty() {
        return None;
    }
    Some(json!({
        "id": id,
        "kind": "user",
        "name": actor.get("name").and_then(Value::as_str).unwrap_or_default()
    }))
}

fn follow_up_activity(activity: &Value, submit: Option<&SubmitPayload>) -> Value {
    let card = follow_up_card(submit);
    json!({
        "type": "message",
        "from": activity.get("recipient").cloned().unwrap_or(Value::Null),
        "recipient": activity.get("from").cloned().unwrap_or(Value::Null),
        "conversation": activity.get("conversation").cloned().unwrap_or(Value::Null),
        "replyToId": activity.get("id").cloned().unwrap_or(Value::Null),
        "attachments": [
            {
                "contentType": "application/vnd.microsoft.card.adaptive",
                "content": card
            }
        ]
    })
}

fn invoke_response_card(submit: Option<&SubmitPayload>) -> Value {
    json!({
        "status": 200,
        "body": {
            "type": "application/vnd.microsoft.card.adaptive",
            "value": follow_up_card(submit)
        }
    })
}

fn follow_up_card(submit: Option<&SubmitPayload>) -> Value {
    let action_id = submit
        .map(|payload| payload.action_id.as_str())
        .unwrap_or("message");
    let submitted_text = submit.map(|payload| payload.text.as_str()).unwrap_or("");
    json!({
        "$schema": "http://adaptivecards.io/schemas/adaptive-card.json",
        "type": "AdaptiveCard",
        "version": "1.5",
        "body": [
            {
                "type": "TextBlock",
                "weight": "Bolder",
                "text": "Teams action received"
            },
            {
                "type": "FactSet",
                "facts": [
                    { "title": "Button", "value": action_id },
                    { "title": "Text", "value": submitted_text }
                ]
            }
        ]
    })
}

fn insert_string(map: &mut Map<String, Value>, key: &str, value: &str) {
    if !value.trim().is_empty() {
        map.insert(key.to_string(), Value::String(value.to_string()));
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn headers() -> String {
        json!({"Authorization": "Bearer test-token"}).to_string()
    }

    fn activity(value: Value) -> Value {
        json!({
            "type": "invoke",
            "id": "activity-1",
            "serviceUrl": "https://smba.trafficmanager.net/emea/",
            "channelId": "msteams",
            "conversation": {"id": "conv-1", "conversationType": "personal"},
            "from": {"id": "user-1", "name": "Ada"},
            "recipient": {"id": "28:bot-id", "name": "Greentic"},
            "channelData": {"tenant": {"id": "tenant-1"}},
            "value": value
        })
    }

    #[test]
    fn submit_payload_extracts_action_and_text() {
        let parsed = extract_submit(&activity(json!({
            "action": "approve",
            "text": "Looks good",
            "msteams": {"type": "messageBack"}
        })))
        .expect("submit payload");
        assert_eq!(parsed.action_id, "approve");
        assert_eq!(parsed.text, "Looks good");
    }

    #[test]
    fn action_execute_payload_extracts_verb_and_data() {
        let parsed = extract_submit(&activity(json!({
            "action": {
                "type": "Action.Execute",
                "verb": "reject",
                "data": {"text": "Needs work"}
            }
        })))
        .expect("submit payload");
        assert_eq!(parsed.action_id, "reject");
        assert_eq!(parsed.text, "Needs work");
    }

    #[test]
    fn handle_activity_returns_follow_up_card() {
        let result = handle_bot_framework_activity(
            &headers(),
            &activity(json!({"action_id": "approve", "inputText": "Ship it"})),
        )
        .expect("activity");
        assert_eq!(result["ok"], true);
        assert_eq!(result["conversation"]["id"], "conv-1");
        assert_eq!(result["events"][0]["metadata"]["action_id"], "approve");
        assert_eq!(result["events"][0]["metadata"]["submitted_text"], "Ship it");
        assert_eq!(
            result["reply_activity"]["attachments"][0]["content"]["body"][1]["facts"][0]["value"],
            "approve"
        );
        assert_eq!(result["invoke_response"]["status"], 200);
    }

    #[test]
    fn conversation_update_members_added_returns_user_entered_event() {
        let result = handle_bot_framework_activity(
            &headers(),
            &json!({
                "type": "conversationUpdate",
                "id": "activity-2",
                "serviceUrl": "https://smba.trafficmanager.net/emea/",
                "channelId": "msteams",
                "conversation": {"id": "conv-1", "conversationType": "personal"},
                "from": {"id": "user-1", "name": "Ada"},
                "recipient": {"id": "28:bot-id", "name": "Greentic"},
                "membersAdded": [
                    {"id": "28:bot-id", "name": "Greentic"},
                    {"id": "user-1", "name": "Ada"}
                ],
                "channelData": {"tenant": {"id": "tenant-1"}}
            }),
        )
        .expect("activity");

        assert_eq!(result["ok"], true);
        assert_eq!(result["events"].as_array().expect("events").len(), 1);
        assert_eq!(result["events"][0]["session_id"], "conv-1");
        assert_eq!(
            result["events"][0]["metadata"]["event_type"],
            "channel.user.entered"
        );
        assert_eq!(result["events"][0]["metadata"]["autoStart"], "true");
        assert_eq!(result["events"][0]["metadata"]["reason"], "members_added");
        assert_eq!(
            result["events"][0]["metadata"]["idempotency_key"],
            "lifecycle.user_entered:teams:tenant-1:conv-1:user-1:members_added"
        );
    }

    #[test]
    fn installation_update_add_returns_user_entered_event() {
        let result = handle_bot_framework_activity(
            &headers(),
            &json!({
                "type": "installationUpdate",
                "id": "activity-3",
                "action": "add",
                "serviceUrl": "https://smba.trafficmanager.net/emea/",
                "channelId": "msteams",
                "conversation": {"id": "conv-1", "conversationType": "personal"},
                "from": {"id": "user-1", "name": "Ada"},
                "recipient": {"id": "28:bot-id", "name": "Greentic"},
                "channelData": {"tenant": {"id": "tenant-1"}}
            }),
        )
        .expect("activity");

        assert_eq!(
            result["events"][0]["metadata"]["event_type"],
            "channel.user.entered"
        );
        assert_eq!(result["events"][0]["metadata"]["reason"], "app_installed");
        assert_eq!(
            result["events"][0]["metadata"]["idempotency_key"],
            "lifecycle.user_entered:teams:tenant-1:conv-1:user-1:app_installed"
        );
    }

    const DOWNLOAD_INFO: &str = "application/vnd.microsoft.teams.file.download.info";

    fn file(name: &str, url: &str, file_type: &str) -> Value {
        json!({"contentType": DOWNLOAD_INFO, "name": name,
               "content": {"downloadUrl": url, "fileType": file_type}})
    }

    fn message(attachments: Value) -> Value {
        json!({"type":"message","id":"a1","text":"t","conversation":{"id":"c"},
               "attachments": attachments})
    }

    #[test]
    fn file_download_info_becomes_a_public_ref_with_a_mime_from_the_extension() {
        let activity = json!({"attachments":[file("plan.pdf","https://contoso.sharepoint.com/d?tempauth=x","pdf")]});
        let pending = teams_pending_attachments(&activity);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].mime_type, "application/pdf");
        assert_eq!(pending[0].name.as_deref(), Some("plan.pdf"));
        assert!(matches!(pending[0].fetch, FetchRef::Public { .. }));
        assert!(pending[0].inline_base64.is_none());
    }

    #[test]
    fn cards_and_http_urls_and_inline_bot_urls_are_not_files() {
        let activity = json!({"attachments":[
            {"contentType":"application/vnd.microsoft.card.adaptive","content":{}},
            file("a.pdf","http://x.sharepoint.com/a","pdf"),
            {"contentType":"image/png","contentUrl":"https://smba.trafficmanager.net/x/v3/attachments/1/views/original"}
        ]});
        assert!(teams_pending_attachments(&activity).is_empty());
    }

    #[test]
    fn normalized_activity_carries_the_refs() {
        let activity = message(json!([file("n.csv", "https://x.sharepoint.com/d", "csv")]));
        let event = normalize_activity(&activity, None);
        assert_eq!(event["attachments"][0]["mime_type"], "text/csv");
        assert_eq!(event["attachments"][0]["name"], "n.csv");
        assert!(event["attachments"][0]["url"].is_null());
        assert_eq!(event["extensions"]["attachment_fetch"][0]["kind"], "public");
        assert_eq!(
            event["extensions"]["attachment_fetch"][0]["url"],
            "https://x.sharepoint.com/d"
        );
        assert!(event["metadata"].get("attachments_dropped").is_none());
    }

    #[test]
    fn hostile_download_urls_are_rejected_and_counted() {
        let long = format!("https://x.sharepoint.com/{}", "a".repeat(3000));
        let bad = [
            "https://contoso.sharepoint.com.evil.test/d",
            "https://evilsharepoint.com/d",
            "https://sharepoint.com.evil.test/d",
            "https://.sharepoint.com/d",
            "http://contoso.sharepoint.com/d",
            "https://user:pw@contoso.sharepoint.com/d",
            "https://contoso.sharepoint.com@evil.test/d",
            "https://10.0.0.1/d",
            "https://127.0.0.1/d",
            "https://[::1]/d",
            "https://169.254.169.254/latest",
            "https://localhost/d",
            "https://contoso.sharepoint.com:8443/d",
            "https://contoso.sharepoint.com\\evil.test/d",
            "ftp://contoso.sharepoint.com/d",
            long.as_str(),
        ];
        for url in bad {
            let event = normalize_activity(&message(json!([file("a.pdf", url, "pdf")])), None);
            assert!(event.get("attachments").is_none(), "kept {url:.80}");
            assert!(event.get("extensions").is_none(), "{url:.80}");
            assert_eq!(event["metadata"]["attachments_dropped"], "1", "{url:.80}");
        }
    }

    #[test]
    fn missing_download_url_or_content_is_dropped_and_counted() {
        let activity = message(json!([
            {"contentType": DOWNLOAD_INFO, "name": "a.pdf", "content": {"fileType": "pdf"}},
            {"contentType": DOWNLOAD_INFO, "name": "b.pdf"},
            {"contentType": DOWNLOAD_INFO, "name": "c.pdf", "content": {"downloadUrl": 7, "fileType": "pdf"}}
        ]));
        assert!(teams_pending_attachments(&activity).is_empty());
        let event = normalize_activity(&activity, None);
        assert!(event.get("extensions").is_none());
        assert_eq!(event["metadata"]["attachments_dropped"], "3");
    }

    #[test]
    fn inline_images_are_skipped_and_counted() {
        let activity = message(json!([
            {"contentType":"image/png","contentUrl":"https://smba.trafficmanager.net/x/v3/attachments/1/views/original","name":"i.png"},
            {"contentType":"image/jpeg","contentUrl":"https://smba.trafficmanager.net/x/v3/attachments/2/views/original"}
        ]));
        let parsed = teams_attachments(&activity);
        assert!(parsed.pending.is_empty());
        assert_eq!(parsed.skipped, 2);
        let event = normalize_activity(&activity, None);
        assert!(event.get("extensions").is_none());
        assert_eq!(event["metadata"]["attachments_dropped"], "2");
    }

    #[test]
    fn mixed_card_file_and_inline_keeps_only_the_file() {
        let activity = message(json!([
            {"contentType":"application/vnd.microsoft.card.adaptive","content":{"type":"AdaptiveCard"}},
            {"contentType":"application/vnd.microsoft.card.hero","content":{}},
            {"contentType":"image/png","contentUrl":"https://smba.trafficmanager.net/x/v3/attachments/1"},
            file("plan.pdf","https://contoso.sharepoint.com/d?tempauth=x","pdf")
        ]));
        let event = normalize_activity(&activity, None);
        let attachments = event["attachments"].as_array().expect("attachments");
        let refs = event["extensions"]["attachment_fetch"]
            .as_array()
            .expect("refs");
        assert_eq!(attachments.len(), 1);
        assert_eq!(refs.len(), 1);
        assert_eq!(attachments[0]["mime_type"], "application/pdf");
        assert_eq!(refs[0]["kind"], "public");
        // The inline image is counted; cards are not files and not drops.
        assert_eq!(event["metadata"]["attachments_dropped"], "1");
    }

    #[test]
    fn the_same_download_url_twice_is_one_file() {
        let url = "https://contoso.sharepoint.com/d?tempauth=x";
        let activity = message(json!([
            file("a.pdf", url, "pdf"),
            file("b.pdf", url, "pdf")
        ]));
        let event = normalize_activity(&activity, None);
        assert_eq!(event["attachments"].as_array().expect("a").len(), 1);
        assert_eq!(event["attachments"][0]["name"], "a.pdf");
        assert!(event["metadata"].get("attachments_dropped").is_none());
    }

    #[test]
    fn names_are_sanitised() {
        let activity = message(json!([file(
            "../x/re\u{202E}fdp.exe\u{0007}",
            "https://contoso.sharepoint.com/d",
            "pdf"
        )]));
        let event = normalize_activity(&activity, None);
        assert_eq!(event["attachments"][0]["name"], "refdp.exe");
    }

    #[test]
    fn seven_files_are_capped_at_five() {
        let files: Vec<Value> = (0..7)
            .map(|i| {
                file(
                    &format!("f{i}.png"),
                    &format!("https://c.sharepoint.com/{i}"),
                    "png",
                )
            })
            .collect();
        let event = normalize_activity(&message(Value::Array(files)), None);
        assert_eq!(event["attachments"].as_array().expect("a").len(), 5);
        assert_eq!(
            event["extensions"]["attachment_fetch"]
                .as_array()
                .expect("r")
                .len(),
            5
        );
        assert_eq!(event["attachments"][4]["name"], "f4.png");
        assert_eq!(event["metadata"]["attachments_dropped"], "2");
    }

    #[test]
    fn file_type_is_a_hint_with_the_name_extension_as_fallback() {
        let activity = message(json!([
            {"contentType": DOWNLOAD_INFO, "name": "notes.MD",
             "content": {"downloadUrl": "https://c.sharepoint.com/1"}},
            file("tool.exe", "https://c.sharepoint.com/2", "exe"),
            file("x.svg", "https://c.sharepoint.com/3", "svg")
        ]));
        let event = normalize_activity(&activity, None);
        assert_eq!(event["attachments"].as_array().expect("a").len(), 1);
        assert_eq!(event["attachments"][0]["mime_type"], "text/markdown");
        assert_eq!(event["metadata"]["attachments_dropped"], "2");
    }

    #[test]
    fn a_message_without_attachments_is_unchanged() {
        let event = normalize_activity(&message(json!([])), None);
        assert!(event.get("attachments").is_none());
        assert!(event.get("extensions").is_none());
        assert!(event["metadata"].get("attachments_dropped").is_none());
        let event = normalize_activity(&json!({"id":"a","conversation":{"id":"c"}}), None);
        assert!(event.get("attachments").is_none());
    }

    #[test]
    fn missing_bearer_is_rejected() {
        let err = handle_bot_framework_activity("{}", &activity(json!({}))).unwrap_err();
        assert!(err.contains("bearer token required"));
    }
}
