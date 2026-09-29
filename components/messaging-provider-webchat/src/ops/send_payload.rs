//! Step 3 of the egress pipeline: persist the encoded payload + emit a bot
//! activity back into the Direct Line conversation so frontend polling can
//! pick it up.
//!
//! For webchat, "send" is a local operation — there is no outbound HTTP call to
//! a third-party API. We write to the host state store and, when a
//! `session_id` is present on the payload, append a bot activity to the
//! stored conversation state.

use base64::{Engine as _, engine::general_purpose};
use greentic_types::messaging::universal_dto::SendPayloadInV1;
use provider_common::helpers::{json_bytes, send_payload_error};
use provider_common::redact;
use provider_common::telemetry::{self, Field, Level, field};
use serde_json::{Value, json};

use crate::PROVIDER_TYPE;
use crate::directline::HostStateStore;
use crate::directline::jwt::DirectLineContext;
use crate::directline::state::{StoredActivity, conversation_key};
use crate::directline::store::StateStore as _;

use super::helpers::{
    extract_text, public_base_url_from_value, route_from_value, tenant_channel_from_value,
    value_as_trimmed_string,
};

/// Watermark/identity metadata captured when a bot reply is appended to a
/// Direct Line conversation. Used to populate the `_greentic` block on
/// `send_payload` responses so the host can fire WebSocket push notifications.
struct GreenticActivityMeta {
    conversation_id: String,
    tenant: String,
    watermark: u64,
}

pub(crate) fn send_payload(input_json: &[u8]) -> Vec<u8> {
    let send_in = match serde_json::from_slice::<SendPayloadInV1>(input_json) {
        Ok(value) => value,
        Err(err) => {
            return send_payload_error(&format!("invalid send_payload input: {err}"), false);
        }
    };
    if send_in.provider_type != PROVIDER_TYPE {
        return send_payload_error("provider type mismatch", false);
    }
    let payload_bytes = match general_purpose::STANDARD.decode(&send_in.payload.body_b64) {
        Ok(bytes) => bytes,
        Err(err) => {
            return send_payload_error(&format!("payload decode failed: {err}"), false);
        }
    };
    let payload: Value = serde_json::from_slice(&payload_bytes).unwrap_or(Value::Null);
    match persist_send_payload(&payload) {
        Ok(meta) => send_payload_success_with_meta(meta.as_ref()),
        Err(err) => {
            try_surface_error_activity(&payload, &err);
            send_payload_error(&err, false)
        }
    }
}

/// Returns `(error_kind, error_message)` when the engine lifted a node failure
/// onto `metadata.error_kind` / `metadata.error_message`.
fn extract_error_envelope(payload: &Value) -> Option<(String, String)> {
    let metadata = payload.get("metadata")?;
    let kind = metadata
        .get("error_kind")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    let message = metadata
        .get("error_message")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    Some((kind.to_string(), message.to_string()))
}

/// Best-effort: append a redacted error card to the conversation pointed at
/// by `payload.session_id` so the chat user sees what failed instead of
/// silently re-prompting. No-op when context is missing.
fn try_surface_error_activity(payload: &Value, error_message: &str) {
    let Some(session_id) = value_as_trimmed_string(payload.get("session_id")) else {
        return;
    };
    let env = value_as_trimmed_string(payload.get("env"));
    let tenant = value_as_trimmed_string(payload.get("tenant"));
    let team = value_as_trimmed_string(payload.get("team"));
    let safe_message = redact::error_message(error_message);
    let _ = append_error_activity_to_conversation(
        &session_id,
        &safe_message,
        "send_payload_error",
        env.as_deref(),
        tenant.as_deref(),
        team.as_deref(),
    );
}

/// Build a `send_payload` success response, optionally including a `_greentic`
/// metadata block when a bot activity was appended to a Direct Line
/// conversation. Underscore-prefixed keys are ignored by Direct Line clients,
/// so adding the block does not break compatibility.
fn send_payload_success_with_meta(meta: Option<&GreenticActivityMeta>) -> Vec<u8> {
    let mut body = json!({
        "ok": true,
        "retryable": false,
    });
    if let Some(meta) = meta
        && let Some(map) = body.as_object_mut()
    {
        map.insert(
            "_greentic".to_string(),
            json!({
                "watermark_bumped": meta.watermark,
                "conversation_id": meta.conversation_id,
                "tenant": meta.tenant,
            }),
        );
    }
    json_bytes(&body)
}

fn persist_send_payload(payload: &Value) -> Result<Option<GreenticActivityMeta>, String> {
    let route = route_from_value(payload);
    let tenant_channel_id = tenant_channel_from_value(payload);
    let key = route
        .clone()
        .or(tenant_channel_id.clone())
        .ok_or_else(|| "route or tenant_channel_id required".to_string())?;
    let text = extract_text(payload);
    let adaptive_card_json = payload
        .get("adaptive_card")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let extensions = payload.get("extensions").cloned();
    let envelope_attachments = payload.get("attachments").filter(|v| v.is_array()).cloned();
    if text.is_empty()
        && adaptive_card_json.is_none()
        && extensions.is_none()
        && envelope_attachments.is_none()
    {
        return Err("text, adaptive_card, attachments, or extensions required".into());
    }

    // Detect a structured flow-error envelope (set by greentic-runner's
    // engine when a user-facing flow node fails). When present, route the
    // activity through the styled error-card path instead of the regular
    // bot reply.
    let error_envelope = extract_error_envelope(payload);

    let mut activity_meta: Option<GreenticActivityMeta> = None;
    if let Some(session_id) = value_as_trimmed_string(payload.get("session_id")) {
        let env = value_as_trimmed_string(payload.get("env"));
        let tenant = value_as_trimmed_string(payload.get("tenant"));
        let team = value_as_trimmed_string(payload.get("team"));

        let append_result = if let Some((error_kind, error_message)) = &error_envelope {
            let safe_message = redact::error_message(error_message);
            append_error_activity_to_conversation(
                &session_id,
                &safe_message,
                error_kind,
                env.as_deref(),
                tenant.as_deref(),
                team.as_deref(),
            )
        } else {
            append_bot_activity_to_conversation(
                &session_id,
                &text,
                adaptive_card_json.as_deref(),
                extensions.as_ref(),
                envelope_attachments.as_ref(),
                env.as_deref(),
                tenant.as_deref(),
                team.as_deref(),
            )
        };

        if let Ok(Some(watermark)) = append_result {
            activity_meta = Some(GreenticActivityMeta {
                conversation_id: session_id,
                tenant: tenant.unwrap_or_else(|| "default".to_string()),
                watermark,
            });
        }
    }

    let public_base_url = public_base_url_from_value(payload);
    let stored = json!({
        "route": route,
        "tenant_channel_id": tenant_channel_id,
        "public_base_url": public_base_url,
        "mode": value_as_trimmed_string(payload.get("mode")).unwrap_or_else(|| "local_queue".to_string()),
        "base_url": value_as_trimmed_string(payload.get("base_url")),
        "text": text,
    });
    let mut state_store = HostStateStore;
    state_store.write(&key, &json_bytes(&stored))?;
    Ok(activity_meta)
}

/// Append a bot-originated activity to the Direct Line conversation state.
/// Uses provided env/tenant context from envelope metadata, falling back to "default".
/// Best-effort: silently returns `Ok(None)` when the conversation does not exist.
///
/// Returns the watermark assigned to the appended activity on success so the
/// caller can emit `_greentic` metadata for WebSocket push notifications.
#[allow(clippy::too_many_arguments)]
fn append_bot_activity_to_conversation(
    conversation_id: &str,
    text: &str,
    adaptive_card_json: Option<&str>,
    extensions: Option<&Value>,
    envelope_attachments: Option<&Value>,
    env: Option<&str>,
    tenant: Option<&str>,
    team: Option<&str>,
) -> Result<Option<u64>, String> {
    let ctx = DirectLineContext {
        env: env.unwrap_or("default").to_string(),
        tenant: tenant.unwrap_or("default").to_string(),
        team: team.map(str::to_string),
    };
    let mut store = HostStateStore;

    let (conv_key, conv_bytes) =
        match find_existing_conversation_state(&mut store, &ctx, conversation_id)? {
            Some(found) => found,
            None => return Ok(None), // conversation not found, skip silently
        };

    let mut conversation: crate::directline::state::ConversationState =
        serde_json::from_slice(&conv_bytes).map_err(|e| e.to_string())?;

    let watermark = conversation.bump_watermark();
    let raw = build_bot_activity_raw(text, adaptive_card_json, extensions, envelope_attachments);

    let activity = StoredActivity {
        id: format!("bot-{watermark}"),
        type_: "message".to_string(),
        text: if text.is_empty() {
            None
        } else {
            Some(text.to_string())
        },
        from: Some("bot".to_string()),
        timestamp: chrono::Utc::now().timestamp_millis(),
        watermark,
        raw,
    };
    conversation.activities.push(activity);

    let updated = serde_json::to_vec(&conversation).map_err(|e| e.to_string())?;
    store.write(&conv_key, &updated)?;
    Ok(Some(watermark))
}

/// Append an "event"-typed activity carrying a redacted error AC card. Returns
/// `Ok(None)` when the conversation isn't found (we never create a stray one
/// just to deliver an error).
fn append_error_activity_to_conversation(
    conversation_id: &str,
    safe_message: &str,
    error_kind: &str,
    env: Option<&str>,
    tenant: Option<&str>,
    team: Option<&str>,
) -> Result<Option<u64>, String> {
    let ctx = DirectLineContext {
        env: env.unwrap_or("default").to_string(),
        tenant: tenant.unwrap_or("default").to_string(),
        team: team.map(str::to_string),
    };
    let mut store = HostStateStore;

    let (conv_key, conv_bytes) =
        match find_existing_conversation_state(&mut store, &ctx, conversation_id)? {
            Some(found) => found,
            None => return Ok(None),
        };

    let mut conversation: crate::directline::state::ConversationState =
        serde_json::from_slice(&conv_bytes).map_err(|e| e.to_string())?;
    let watermark = conversation.bump_watermark();
    let card = build_error_adaptive_card(safe_message, error_kind);
    let card_json = serde_json::to_string(&card).unwrap_or_else(|_| "{}".to_string());
    let raw = build_bot_activity_raw(safe_message, Some(&card_json), None, None);

    let activity = StoredActivity {
        id: format!("error-{watermark}"),
        // `event` type avoids the bot-echo loop and one-shot delivery.
        type_: "event".to_string(),
        text: Some(safe_message.to_string()),
        from: Some("bot".to_string()),
        timestamp: chrono::Utc::now().timestamp_millis(),
        watermark,
        raw,
    };
    conversation.activities.push(activity);

    let updated = serde_json::to_vec(&conversation).map_err(|e| e.to_string())?;
    store.write(&conv_key, &updated)?;
    Ok(Some(watermark))
}

/// Adaptive Card body for the error activity. Attention-styled headline +
/// the redacted detail + a small `kind:` tag.
fn build_error_adaptive_card(safe_message: &str, error_kind: &str) -> Value {
    json!({
        "type": "AdaptiveCard",
        "$schema": "http://adaptivecards.io/schemas/adaptive-card.json",
        "version": "1.5",
        "body": [
            {
                "type": "TextBlock",
                "text": "Something went wrong",
                "weight": "Bolder",
                "size": "Medium",
                "color": "Attention"
            },
            {
                "type": "TextBlock",
                "text": safe_message,
                "wrap": true
            },
            {
                "type": "TextBlock",
                "text": format!("kind: {error_kind}"),
                "isSubtle": true,
                "size": "Small",
                "spacing": "Small"
            }
        ]
    })
}

/// Try to locate an existing conversation state under any plausible context
/// variant. Walks `candidate_conversation_contexts` and returns the first key
/// that resolves. Logs the keys tried when none match — silent misses here
/// translate directly into "/activities is empty" symptoms in the SPA.
pub(super) fn find_existing_conversation_state<S: crate::directline::store::StateStore>(
    store: &mut S,
    ctx: &DirectLineContext,
    conversation_id: &str,
) -> Result<Option<(String, Vec<u8>)>, String> {
    let mut tried_keys: Vec<String> = Vec::new();
    for candidate_ctx in candidate_conversation_contexts(ctx) {
        let key = conversation_key(&candidate_ctx, conversation_id);
        if let Some(bytes) = store.read(&key)? {
            return Ok(Some((key, bytes)));
        }
        tried_keys.push(key);
    }
    let conv_redacted = redact::user_id(conversation_id);
    let team_str = ctx.team.as_deref().unwrap_or("<none>");
    let tried = tried_keys.join(",");
    telemetry::emit(
        Level::Warn,
        PROVIDER_TYPE,
        "conversation lookup miss",
        &[
            Field {
                key: field::CONVERSATION_ID,
                value: &conv_redacted,
            },
            Field {
                key: "ctx_env",
                value: ctx.env.as_str(),
            },
            Field {
                key: field::TENANT,
                value: ctx.tenant.as_str(),
            },
            Field {
                key: "ctx_team",
                value: team_str,
            },
            Field {
                key: "tried_keys",
                value: &tried,
            },
        ],
    );
    Ok(None)
}

/// Generate plausible `DirectLineContext` candidates for a conversation
/// lookup. Covers cases where the JWT-side ctx (used at conversation create)
/// differs from the egress-side ctx (rebuilt from envelope metadata) on team
/// only — operator-side path injection sometimes adds team="default" while
/// the JWT context had team=None, or vice versa.
fn candidate_conversation_contexts(ctx: &DirectLineContext) -> Vec<DirectLineContext> {
    let mut contexts: Vec<DirectLineContext> = Vec::new();
    contexts.push(ctx.clone());

    contexts.push(DirectLineContext {
        env: ctx.env.clone(),
        tenant: ctx.tenant.clone(),
        team: None,
    });

    contexts.push(DirectLineContext {
        env: ctx.env.clone(),
        tenant: ctx.tenant.clone(),
        team: Some("default".to_string()),
    });

    if let Some(team) = ctx.team.as_deref() {
        let trimmed = team.trim();
        if !trimmed.is_empty() && trimmed != team {
            contexts.push(DirectLineContext {
                env: ctx.env.clone(),
                tenant: ctx.tenant.clone(),
                team: Some(trimmed.to_string()),
            });
        }
    }

    contexts.dedup();
    contexts
}

/// Build the raw DirectLine activity JSON for a bot message, merging any
/// `extensions` fields (attachments, channelData, entities, etc.) and the
/// envelope's top-level `attachments` field back into their DirectLine-native
/// camelCase form.
///
/// Merge order for attachments: legacy AC (metadata) → envelope.attachments
/// (top-level typed field) → extensions.adaptive_card → extensions.attachments.
///
/// Pure function — no state I/O — suitable for unit testing the merge logic.
fn build_bot_activity_raw(
    text: &str,
    adaptive_card_json: Option<&str>,
    extensions: Option<&Value>,
    envelope_attachments: Option<&Value>,
) -> Value {
    let mut raw = json!({
        "type": "message",
        "from": {"id": "bot", "name": "Bot", "role": "bot"},
    });
    if !text.is_empty() {
        raw["text"] = Value::String(text.to_string());
    }

    let mut attachments: Vec<Value> = Vec::new();
    if let Some(ac_json) = adaptive_card_json
        && let Ok(ac_value) = serde_json::from_str::<Value>(ac_json)
    {
        attachments.push(json!({
            "contentType": "application/vnd.microsoft.card.adaptive",
            "content": ac_value,
        }));
    }

    if let Some(Value::Array(env_atts)) = envelope_attachments {
        attachments.extend(env_atts.clone());
    }

    if let Some(ext) = extensions
        && let Some(ext_obj) = ext.as_object()
    {
        if adaptive_card_json.is_none()
            && let Some(ac) = ext_obj.get("adaptive_card")
        {
            attachments.push(json!({
                "contentType": "application/vnd.microsoft.card.adaptive",
                "content": ac.clone(),
            }));
        }
        if let Some(Value::Array(ext_atts)) = ext_obj.get("attachments") {
            attachments.extend(ext_atts.clone());
        }
        let passthroughs: &[(&str, &str)] = &[
            ("entities", "entities"),
            ("name", "name"),
            ("input_hint", "inputHint"),
            ("speak", "speak"),
            ("suggested_actions", "suggestedActions"),
        ];
        if let Some(channel_data) = ext_obj
            .get("channel_data")
            .and_then(sanitize_outbound_channel_data)
        {
            raw["channelData"] = channel_data;
        }
        for (src, dst) in passthroughs {
            if let Some(v) = ext_obj.get(*src)
                && !v.is_null()
            {
                raw[*dst] = v.clone();
            }
        }
    }

    if !attachments.is_empty() {
        raw["attachments"] = Value::Array(attachments);
    }
    raw
}

fn sanitize_outbound_channel_data(value: &Value) -> Option<Value> {
    let mut channel_data = match value {
        Value::Object(map) => map.clone(),
        other if !other.is_null() => return Some(other.clone()),
        _ => return None,
    };

    // These keys are useful on the inbound Direct Line activity, but if they
    // are echoed on a bot response the webchat client can treat the response
    // as a postBack/request artefact instead of rendering the card.
    for key in [
        "postBack",
        "clientActivityID",
        "clientActivityId",
        "attachmentSizes",
    ] {
        channel_data.remove(key);
    }

    if channel_data.is_empty() {
        None
    } else {
        Some(Value::Object(channel_data))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use greentic_types::messaging::universal_dto::ProviderPayloadV1;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn parse_result(bytes: Vec<u8>) -> Value {
        serde_json::from_slice(&bytes).expect("json result")
    }

    fn send_payload_input(provider_type: &str, body_b64: &str) -> Vec<u8> {
        let input = SendPayloadInV1 {
            provider_type: provider_type.to_string(),
            tenant_id: None,
            auth_user: None,
            payload: ProviderPayloadV1 {
                content_type: "application/json".to_string(),
                body_b64: body_b64.to_string(),
                metadata: BTreeMap::new(),
            },
        };
        serde_json::to_vec(&input).expect("input")
    }

    // This file is #[path]-included by every webchat variant crate, so these two
    // run once per variant and pin each one's own PROVIDER_TYPE past the guard.
    #[test]
    fn send_payload_accepts_this_variant_provider_type() {
        let body = parse_result(send_payload(&send_payload_input(
            PROVIDER_TYPE,
            "not base64",
        )));

        assert_eq!(body["ok"], false);
        assert_ne!(
            body["message"], "provider type mismatch",
            "{PROVIDER_TYPE} must clear its own guard"
        );
        assert!(
            body["message"]
                .as_str()
                .unwrap_or_default()
                .contains("payload decode failed")
        );
    }

    #[test]
    fn send_payload_rejects_foreign_provider_type() {
        let body = parse_result(send_payload(&send_payload_input(
            "messaging.slack.api",
            "not base64",
        )));

        assert_eq!(body["ok"], false);
        assert_eq!(body["message"], "provider type mismatch");
    }

    #[test]
    fn build_bot_activity_raw_plain_text_only() {
        let raw = build_bot_activity_raw("hello", None, None, None);
        assert_eq!(raw["type"], "message");
        assert_eq!(raw["text"], "hello");
        assert_eq!(raw["from"]["id"], "bot");
        assert!(raw.get("attachments").is_none());
    }

    #[test]
    fn build_bot_activity_raw_adaptive_card_via_legacy_metadata() {
        // Upstream still writes metadata["adaptive_card"] as JSON string.
        let ac_json = r#"{"type":"AdaptiveCard","body":[{"type":"TextBlock","text":"hi"}]}"#;
        let raw = build_bot_activity_raw("hi", Some(ac_json), None, None);

        let attachments = raw["attachments"].as_array().expect("attachments array");
        assert_eq!(attachments.len(), 1);
        assert_eq!(
            attachments[0]["contentType"],
            "application/vnd.microsoft.card.adaptive"
        );
        assert_eq!(attachments[0]["content"]["type"], "AdaptiveCard");
    }

    #[test]
    fn build_bot_activity_raw_adaptive_card_via_extensions() {
        // New path: RAG component writes extensions["adaptive_card"] as typed Value.
        let extensions = json!({
            "adaptive_card": {"type": "AdaptiveCard", "body": [{"type": "TextBlock", "text": "hi"}]},
        });
        let raw = build_bot_activity_raw("hi", None, Some(&extensions), None);

        let attachments = raw["attachments"].as_array().expect("attachments array");
        assert_eq!(attachments.len(), 1);
        assert_eq!(
            attachments[0]["contentType"],
            "application/vnd.microsoft.card.adaptive"
        );
        assert_eq!(attachments[0]["content"]["type"], "AdaptiveCard");
    }

    #[test]
    fn build_bot_activity_raw_merges_ac_from_metadata_and_native_attachments_from_extensions() {
        // Legacy AC in metadata + provider-native attachments in extensions.
        let ac_json = r#"{"type":"AdaptiveCard","body":[]}"#;
        let extensions = json!({
            "attachments": [
                {"contentType": "application/vnd.microsoft.card.hero", "content": {"title": "Hero"}}
            ]
        });
        let raw = build_bot_activity_raw("", Some(ac_json), Some(&extensions), None);

        let attachments = raw["attachments"].as_array().expect("attachments array");
        assert_eq!(attachments.len(), 2, "AC + hero card expected");
        assert_eq!(
            attachments[0]["contentType"],
            "application/vnd.microsoft.card.adaptive"
        );
        assert_eq!(
            attachments[1]["contentType"],
            "application/vnd.microsoft.card.hero"
        );
    }

    #[test]
    fn build_bot_activity_raw_materializes_all_directline_fields() {
        // Full scenario: RAG component emits AC + citations (via channelData.rag) +
        // all DirectLine Bot Framework fields, expecting them preserved verbatim.
        let extensions = json!({
            "adaptive_card": {"type": "AdaptiveCard", "body": []},
            "channel_data": {"rag": {"type": "rag", "citations": [{"index": 1, "doc": "Metro Design v3.2"}]}, "feature": "x"},
            "entities": [{"type": "mention", "text": "@bot"}],
            "input_hint": "acceptingInput",
            "speak": "hello there",
            "suggested_actions": {"actions": [{"type": "imBack", "title": "Yes", "value": "yes"}]},
            "name": "event/custom",
        });

        let raw = build_bot_activity_raw("Based on your docs...", None, Some(&extensions), None);

        // Text preserved.
        assert_eq!(raw["text"], "Based on your docs...");
        // AC wrapped as attachment.
        let atts = raw["attachments"].as_array().unwrap();
        assert_eq!(
            atts[0]["contentType"],
            "application/vnd.microsoft.card.adaptive"
        );
        // Snake_case extensions keys → DirectLine camelCase.
        assert_eq!(
            raw["channelData"],
            json!({
                "rag": {"type": "rag", "citations": [{"index": 1, "doc": "Metro Design v3.2"}]},
                "feature": "x"
            })
        );
        assert_eq!(raw["entities"][0]["type"], "mention");
        assert_eq!(raw["inputHint"], "acceptingInput");
        assert_eq!(raw["speak"], "hello there");
        assert_eq!(raw["suggestedActions"]["actions"][0]["value"], "yes");
        assert_eq!(raw["name"], "event/custom");
    }

    #[test]
    fn build_bot_activity_raw_rag_citations_round_trip() {
        // Regression test for TASK-082 Bug 3 — RAG component emits citations
        // via extensions["channel_data"]["rag"], they must survive into the
        // DirectLine activity's channelData field exactly as emitted.
        let extensions = json!({
            "adaptive_card": {"type": "AdaptiveCard", "body": []},
            "channel_data": {
                "rag": {
                    "type": "rag",
                    "citations": [
                        {"index": 1, "doc": "Metro Design v3.2", "source_file": "metro-design-v3.2.pdf", "excerpt": "..."},
                        {"index": 2, "doc": "Capacity Review 2026", "source_file": "capacity-2026.pdf", "excerpt": "..."}
                    ]
                }
            }
        });

        let raw = build_bot_activity_raw("answer", None, Some(&extensions), None);

        let citations = raw
            .pointer("/channelData/rag/citations")
            .and_then(|v| v.as_array())
            .expect("citations preserved under channelData.rag");
        assert_eq!(citations.len(), 2);
        assert_eq!(citations[0]["index"], 1);
        assert_eq!(citations[0]["source_file"], "metro-design-v3.2.pdf");
        assert_eq!(citations[1]["index"], 2);
    }

    #[test]
    fn build_bot_activity_raw_skips_null_extension_fields() {
        let extensions = json!({
            "channel_data": null,
            "entities": null,
            "speak": "keep me",
        });
        let raw = build_bot_activity_raw("x", None, Some(&extensions), None);
        assert!(raw.get("channelData").is_none());
        assert!(raw.get("entities").is_none());
        assert_eq!(raw["speak"], "keep me");
    }

    #[test]
    fn build_bot_activity_raw_strips_click_artifacts_from_channel_data() {
        let extensions = json!({
            "channel_data": {
                "postBack": true,
                "clientActivityID": "abc123",
                "attachmentSizes": [],
                "feature": "menu",
                "rag": {"type": "rag", "citations": [{"index": 1, "doc": "Metro Design v3.2"}]}
            }
        });
        let raw = build_bot_activity_raw("submenu", None, Some(&extensions), None);
        assert_eq!(raw["channelData"]["feature"], "menu");
        assert_eq!(
            raw["channelData"]["rag"]["citations"][0]["doc"],
            "Metro Design v3.2"
        );
        assert!(raw["channelData"].get("postBack").is_none());
        assert!(raw["channelData"].get("clientActivityID").is_none());
        assert!(raw["channelData"].get("attachmentSizes").is_none());
    }

    #[test]
    fn build_bot_activity_raw_drops_channel_data_when_only_click_artifacts_remain() {
        let extensions = json!({
            "channel_data": {
                "postBack": true,
                "clientActivityID": "abc123"
            }
        });
        let raw = build_bot_activity_raw("submenu", None, Some(&extensions), None);
        assert!(raw.get("channelData").is_none());
    }

    #[test]
    fn build_bot_activity_raw_forwards_envelope_attachments() {
        // Regression test for TASK-082 Bug 3 follow-up — envelope's top-level
        // `attachments: Vec<Attachment>` field must reach the DirectLine activity.
        let envelope_attachments = json!([
            {
                "contentType": "image/png",
                "contentUrl": "https://cdn.example.com/diagram.png",
                "name": "diagram.png",
            }
        ]);
        let raw = build_bot_activity_raw("see image", None, None, Some(&envelope_attachments));

        let attachments = raw["attachments"].as_array().expect("attachments array");
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0]["contentType"], "image/png");
        assert_eq!(
            attachments[0]["contentUrl"],
            "https://cdn.example.com/diagram.png"
        );
    }

    #[test]
    fn build_bot_activity_raw_merges_envelope_attachments_with_legacy_ac_and_extensions() {
        // All three sources present — expect legacy AC first, envelope attachments
        // second, extensions.attachments last. Deterministic order matters for
        // clients that rely on card-ordering semantics.
        let ac_json = r#"{"type":"AdaptiveCard","body":[]}"#;
        let envelope_attachments = json!([
            {"contentType": "image/png", "contentUrl": "https://e/1.png"}
        ]);
        let extensions = json!({
            "attachments": [
                {"contentType": "application/vnd.microsoft.card.hero", "content": {"title": "H"}}
            ]
        });

        let raw = build_bot_activity_raw(
            "multi",
            Some(ac_json),
            Some(&extensions),
            Some(&envelope_attachments),
        );

        let attachments = raw["attachments"].as_array().expect("attachments array");
        assert_eq!(attachments.len(), 3);
        assert_eq!(
            attachments[0]["contentType"],
            "application/vnd.microsoft.card.adaptive"
        );
        assert_eq!(attachments[1]["contentType"], "image/png");
        assert_eq!(
            attachments[2]["contentType"],
            "application/vnd.microsoft.card.hero"
        );
    }

    #[test]
    fn build_bot_activity_raw_ignores_non_array_envelope_attachments() {
        // Defensive: if upstream emits a malformed non-array value, drop silently.
        let envelope_attachments = json!({"this": "is not an array"});
        let raw = build_bot_activity_raw("x", None, None, Some(&envelope_attachments));
        assert!(raw.get("attachments").is_none());
    }

    #[test]
    fn append_context_uses_team_from_payload() {
        assert_eq!(
            value_as_trimmed_string(json!({"team": "default"}).get("team")).as_deref(),
            Some("default")
        );
        assert_eq!(
            value_as_trimmed_string(json!({"team": "  "}).get("team")),
            None
        );
    }

    #[test]
    fn error_adaptive_card_has_expected_shape() {
        let card = build_error_adaptive_card("downstream 502", "send_payload_error");
        assert_eq!(card["type"], "AdaptiveCard");
        let body = card["body"].as_array().expect("body array");
        assert_eq!(body.len(), 3);
        assert_eq!(body[0]["color"], "Attention");
        assert_eq!(body[0]["weight"], "Bolder");
        assert_eq!(body[1]["text"], "downstream 502");
        assert_eq!(body[1]["wrap"], true);
        assert_eq!(body[2]["text"], "kind: send_payload_error");
        assert_eq!(body[2]["isSubtle"], true);
    }

    #[test]
    fn try_surface_error_activity_no_session_id_is_silent_noop() {
        try_surface_error_activity(&json!({}), "input decode failed");
        try_surface_error_activity(&json!({"session_id": "  "}), "blank session");
    }

    #[test]
    fn extract_error_envelope_recognises_engine_shape() {
        // What greentic-runner's engine writes when a user-facing flow node
        // fails: `metadata.error_kind` + `metadata.error_message`.
        let payload = json!({
            "text": "Something went wrong while running this step: 401",
            "metadata": {
                "error_kind": "flow_node_failed",
                "error_message": "weatherapi returned 401 Unauthorized",
                "node_id": "call_weather",
            }
        });
        let (kind, message) = extract_error_envelope(&payload).expect("envelope present");
        assert_eq!(kind, "flow_node_failed");
        assert_eq!(message, "weatherapi returned 401 Unauthorized");
    }

    #[test]
    fn extract_error_envelope_returns_none_for_regular_replies() {
        // Normal bot replies must not be confused for errors.
        assert!(extract_error_envelope(&json!({"text": "hello"})).is_none());
        assert!(extract_error_envelope(&json!({"metadata": {}})).is_none());
        // Empty values fall through too: a real envelope must have both
        // fields and they must be non-empty.
        assert!(
            extract_error_envelope(&json!({
                "metadata": {"error_kind": "", "error_message": "x"}
            }))
            .is_none()
        );
        assert!(
            extract_error_envelope(&json!({
                "metadata": {"error_kind": "x", "error_message": "  "}
            }))
            .is_none()
        );
    }
}
