//! `encode` op for the Telegram provider.
//!
//! Responsibilities:
//! 1. Decode the host `encode` CBOR/JSON request into a
//!    [`ChannelMessageEnvelope`].
//! 2. If an Adaptive Card is attached, delegate to
//!    [`super::ac_to_html::ac_to_telegram`] to produce HTML + actions + image
//!    URLs + pending inputs, stashing them in metadata for `send.rs` to pick
//!    up later.
//! 3. Normalise all `tg_*` metadata fields (strip operator quoting, map URL
//!    suffixes, merge lat/lon into `tg_location`, classify attachments by MIME
//!    type).
//! 4. Re-serialise into a [`ProviderPayloadV1`] the runner can feed into
//!    `send_payload`.

use base64::{Engine, engine::general_purpose::STANDARD};
use greentic_types::messaging::universal_dto::ProviderPayloadV1;
use provider_common::helpers::{decode_encode_message, encode_error, json_bytes};
use serde_json::json;
use std::collections::BTreeMap;

use super::ac_helpers::compact_callback_data;
use super::ac_to_html::ac_to_telegram;
use super::form_reply::{FormMarker, embed_marker};

pub(crate) fn encode_op(input_json: &[u8]) -> Vec<u8> {
    let mut envelope = match decode_encode_message(input_json) {
        Ok(value) => value,
        Err(err) => return encode_error(&err),
    };

    // If the message carries an Adaptive Card, convert it to rich Telegram
    // content: HTML text, inline keyboard buttons, images for sendPhoto.
    if let Some(ac_raw) = provider_common::helpers::resolve_adaptive_card(&envelope)
        .map(|v| serde_json::to_string(&v).unwrap_or_default())
        .as_deref()
        && let Some(content) = ac_to_telegram(ac_raw)
    {
        envelope.text = Some(content.html);
        envelope
            .metadata
            .insert("parse_mode".to_string(), "HTML".to_string());
        if !content.actions.is_empty() {
            let aj = serde_json::to_string(&content.actions).unwrap_or_default();
            envelope.metadata.insert("ac_actions".to_string(), aj);
        }
        if !content.images.is_empty() {
            let ij = serde_json::to_string(&content.images).unwrap_or_default();
            envelope.metadata.insert("ac_images".to_string(), ij);
        }
        if !content.inputs.is_empty() {
            // `ac_pending_inputs` is consumed by `ops::send::handle_send` via
            // `ac_helpers::has_pending_text_inputs` / `first_input_placeholder`
            // to drive Telegram `ForceReply` mode for Adaptive Card text inputs.
            // See `ops::ac_inputs` top-of-file docs for the full contract.
            let ij = serde_json::to_string(&content.inputs).unwrap_or_default();
            envelope
                .metadata
                .insert("ac_pending_inputs".to_string(), ij);
            // Store the submit action data so ingest_http can auto-submit
            // after all text inputs are collected.
            let submit_data = content.actions.iter().find_map(|a| a.get("data")).cloned();
            if let Some(data) = &submit_data {
                envelope
                    .metadata
                    .insert("ac_submit_data".to_string(), data.to_string());
            }
            // Metadata does not survive to the inbound reply, but the message
            // does: anchor what the reply answers on the first pencil so
            // `ingest_http` can submit the card (see `form_reply`).
            let marker = FormMarker::for_inputs(
                &content.inputs,
                submit_data.as_ref().map(compact_callback_data),
            );
            if let (Some(marker), Some(text)) = (marker, envelope.text.as_mut()) {
                *text = embed_marker(text, &marker);
            }
        }
    }

    // ── Map tg_* metadata fields and attachments to internal keys ──

    // 1. Strip operator double-quotes from all tg_* values
    let tg_keys: Vec<String> = envelope
        .metadata
        .keys()
        .filter(|k| k.starts_with("tg_"))
        .cloned()
        .collect();
    for key in tg_keys {
        if let Some(val) = envelope.metadata.get_mut(&key) {
            let trimmed = val.trim_matches('"').to_string();
            *val = trimmed;
        }
    }

    // 2. Map URL-suffixed keys to internal keys
    for (src, dst) in [
        ("tg_video_url", "tg_video"),
        ("tg_audio_url", "tg_audio"),
        ("tg_document_url", "tg_document"),
        ("tg_voice_url", "tg_voice"),
        ("tg_animation_url", "tg_animation"),
        ("tg_sticker_url", "tg_sticker"),
        ("tg_photo_url", "tg_photo"),
    ] {
        if let Some(val) = envelope.metadata.get(src).cloned()
            && !val.is_empty()
        {
            envelope.metadata.entry(dst.to_string()).or_insert(val);
        }
    }

    // 3. Build location from lat/lon metadata
    {
        let lat = envelope.metadata.get("tg_location_latitude").cloned();
        let lon = envelope.metadata.get("tg_location_longitude").cloned();
        let loc_name = envelope.metadata.get("tg_location_name").cloned();
        let loc_addr = envelope.metadata.get("tg_location_address").cloned();
        if let (Some(lat), Some(lon)) = (lat, lon)
            && !lat.is_empty()
            && !lon.is_empty()
            && !envelope.metadata.contains_key("tg_location")
        {
            let mut loc = json!({"latitude": lat, "longitude": lon});
            if let Some(name) = loc_name
                && !name.is_empty()
            {
                loc["name"] = json!(name);
            }
            if let Some(addr) = loc_addr
                && !addr.is_empty()
            {
                loc["address"] = json!(addr);
            }
            envelope.metadata.insert(
                "tg_location".to_string(),
                serde_json::to_string(&loc).unwrap_or_default(),
            );
        }
    }

    // 4. Map attachments by mime_type (metadata takes precedence)
    let att_fields: Vec<(String, String)> = envelope
        .attachments
        .iter()
        .filter_map(|att| {
            let url = att.url.clone()?;
            let mt = att.mime_type.to_lowercase();
            let key = if mt.starts_with("video/") {
                "tg_video"
            } else if mt == "audio/ogg" || mt.starts_with("audio/ogg;") || mt == "audio/opus" {
                "tg_voice"
            } else if mt.starts_with("audio/") {
                "tg_audio"
            } else if mt == "image/webp" {
                "tg_sticker"
            } else if mt == "image/gif" {
                "tg_animation"
            } else if mt.starts_with("image/") {
                "tg_photo"
            } else {
                "tg_document"
            };
            Some((key.to_string(), url))
        })
        .collect();
    for (k, v) in att_fields {
        envelope.metadata.entry(k).or_insert(v);
    }

    let has_text = envelope
        .text
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .is_some();
    if !has_text {
        envelope.text = Some("universal telegram payload".to_string());
    }
    let body_bytes = serde_json::to_vec(&envelope).unwrap_or_else(|_| b"{}".to_vec());
    let payload = ProviderPayloadV1 {
        content_type: "application/json".to_string(),
        body_b64: STANDARD.encode(&body_bytes),
        metadata: BTreeMap::new(),
    };
    json_bytes(&json!({"ok": true, "payload": payload}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;
    use greentic_types::{
        Attachment, ChannelMessageEnvelope, Destination, EnvId, MessageMetadata, TenantCtx,
        TenantId,
    };
    use serde_json::Value;

    fn envelope(metadata: MessageMetadata, attachments: Vec<Attachment>) -> ChannelMessageEnvelope {
        ChannelMessageEnvelope {
            id: "msg-1".to_string(),
            tenant: TenantCtx::new(
                EnvId::try_from("dev").expect("env"),
                TenantId::try_from("tenant").expect("tenant"),
            ),
            channel: "telegram".to_string(),
            session_id: "session-1".to_string(),
            reply_scope: None,
            from: None,
            to: vec![Destination {
                id: "12345".to_string(),
                kind: Some("chat".to_string()),
            }],
            correlation_id: None,
            text: None,
            attachments,
            metadata,
            extensions: Default::default(),
        }
    }

    fn encoded_envelope(input: Value) -> ChannelMessageEnvelope {
        let result: Value = serde_json::from_slice(&encode_op(input.to_string().as_bytes()))
            .expect("encode result");
        assert_eq!(result["ok"], true);
        let payload = result["payload"].as_object().expect("payload");
        let body_b64 = payload
            .get("body_b64")
            .and_then(Value::as_str)
            .expect("body_b64");
        let body = STANDARD.decode(body_b64).expect("payload body");
        serde_json::from_slice(&body).expect("encoded envelope")
    }

    #[test]
    fn encode_normalizes_telegram_metadata_and_location() {
        let mut metadata = MessageMetadata::new();
        metadata.insert(
            "tg_photo_url".to_string(),
            "\"https://cdn/photo.jpg\"".to_string(),
        );
        metadata.insert("tg_location_latitude".to_string(), "51.5".to_string());
        metadata.insert("tg_location_longitude".to_string(), "-0.12".to_string());
        metadata.insert("tg_location_name".to_string(), "Office".to_string());

        let out = encoded_envelope(json!({ "message": envelope(metadata, vec![]) }));

        assert_eq!(out.metadata["tg_photo_url"], "https://cdn/photo.jpg");
        assert_eq!(out.metadata["tg_photo"], "https://cdn/photo.jpg");
        let location: Value = serde_json::from_str(&out.metadata["tg_location"]).expect("location");
        assert_eq!(location["latitude"], "51.5");
        assert_eq!(location["longitude"], "-0.12");
        assert_eq!(location["name"], "Office");
    }

    #[test]
    fn encode_classifies_attachments_without_overriding_metadata() {
        let mut metadata = MessageMetadata::new();
        metadata.insert(
            "tg_photo".to_string(),
            "https://existing/photo.jpg".to_string(),
        );
        let attachments = vec![
            Attachment {
                mime_type: "image/png".to_string(),
                url: Some("https://cdn/new.png".to_string()),
                content: None,
                name: None,
                size_bytes: None,
            },
            Attachment {
                mime_type: "audio/ogg".to_string(),
                url: Some("https://cdn/voice.ogg".to_string()),
                content: None,
                name: None,
                size_bytes: None,
            },
            Attachment {
                mime_type: "application/pdf".to_string(),
                url: Some("https://cdn/file.pdf".to_string()),
                content: None,
                name: None,
                size_bytes: None,
            },
        ];

        let out = encoded_envelope(json!(envelope(metadata, attachments)));

        assert_eq!(out.metadata["tg_photo"], "https://existing/photo.jpg");
        assert_eq!(out.metadata["tg_voice"], "https://cdn/voice.ogg");
        assert_eq!(out.metadata["tg_document"], "https://cdn/file.pdf");
        assert_eq!(out.text.as_deref(), Some("universal telegram payload"));
    }

    #[test]
    fn encode_reports_invalid_input_shape() {
        let result: Value = serde_json::from_slice(&encode_op(br#"{"message":{"bad":true}}"#))
            .expect("encode result");

        assert_eq!(result["ok"], false);
        assert!(
            result["error"]
                .as_str()
                .unwrap_or_default()
                .contains("invalid encode input")
        );
    }
}
