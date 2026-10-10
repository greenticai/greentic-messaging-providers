//! Slack modal support — `views.open` + `view_submission` handling.
//!
//! Adaptive Cards that contain `Input.*` fields cannot render inline in Slack
//! messages, so when such a card is triggered via an `Action.Submit` button we
//! open a Slack modal. On submit, [`handle_view_submission`] converts the
//! modal state back into a channel envelope with the collected input values.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use greentic_types::messaging::universal_dto::HttpOutV1;
use provider_common::http_compat::{http_out_error, http_out_v1_bytes};
use serde_json::{Value, json};
use std::collections::BTreeMap;

use super::{build_slack_envelope, set_verified_caller};
use crate::bindings::greentic::http::http_client as client;
use crate::config::get_secret_string;
use crate::{DEFAULT_API_BASE, DEFAULT_BOT_TOKEN_KEY};

/// Build a Slack modal view from AC input field specs (Input.Text + Input.ChoiceSet)
/// and open it via the Slack `views.open` API using the `trigger_id` from the interaction.
pub(super) fn open_slack_modal(
    trigger_id: &str,
    action_data: &Value,
    channel: Option<&str>,
) -> Vec<u8> {
    let inputs = action_data
        .get("ac_modal_inputs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    // Build modal input blocks from the AC input specs.
    let mut modal_blocks: Vec<Value> = Vec::new();
    for input in &inputs {
        let id = input.get("id").and_then(Value::as_str).unwrap_or("input");
        let label = input.get("label").and_then(Value::as_str).unwrap_or(id);
        let placeholder = input
            .get("placeholder")
            .and_then(Value::as_str)
            .unwrap_or("");
        let is_required = input
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let input_type = input
            .get("input_type")
            .and_then(Value::as_str)
            .unwrap_or("text");

        let element = match input_type {
            "choice" => {
                // Build static_select / multi_static_select from choices.
                let choices = input
                    .get("choices")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let options: Vec<Value> = choices
                    .iter()
                    .take(100)
                    .filter_map(|c| {
                        let title = c.get("title").and_then(Value::as_str)?;
                        let value = c.get("value").and_then(Value::as_str).unwrap_or(title);
                        Some(json!({
                            "text": { "type": "plain_text", "text": title.chars().take(75).collect::<String>() },
                            "value": value.chars().take(75).collect::<String>()
                        }))
                    })
                    .collect();
                if options.is_empty() {
                    continue;
                }
                let is_multi = input.get("multi").and_then(Value::as_bool).unwrap_or(false);
                let select_type = if is_multi {
                    "multi_static_select"
                } else {
                    "static_select"
                };
                let mut el = json!({
                    "type": select_type,
                    "action_id": id,
                    "options": options,
                });
                if !placeholder.is_empty() {
                    el.as_object_mut().unwrap().insert(
                        "placeholder".into(),
                        json!({"type": "plain_text", "text": placeholder.chars().take(150).collect::<String>()}),
                    );
                }
                el
            }
            _ => {
                // plain_text_input for Input.Text
                let is_multiline = input
                    .get("multiline")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let mut el = json!({
                    "type": "plain_text_input",
                    "action_id": id,
                    "multiline": is_multiline,
                });
                if !placeholder.is_empty() {
                    el.as_object_mut().unwrap().insert(
                        "placeholder".into(),
                        json!({"type": "plain_text", "text": placeholder.chars().take(150).collect::<String>()}),
                    );
                }
                el
            }
        };

        let block = json!({
            "type": "input",
            "block_id": format!("ac_input_{id}"),
            "optional": !is_required,
            "label": { "type": "plain_text", "text": label.chars().take(48).collect::<String>() },
            "element": element,
        });
        modal_blocks.push(block);
    }

    if modal_blocks.is_empty() {
        // Fallback: no inputs found, just ack.
        let out = HttpOutV1 {
            status: 200,
            headers: Vec::new(),
            body_b64: STANDARD.encode(b"{}"),
            events: vec![],
        };
        return http_out_v1_bytes(&out);
    }

    // Preserve the original action data (minus modal fields) as private_metadata
    // so we can forward it when the modal is submitted.
    let mut forward_data = action_data.clone();
    if let Some(obj) = forward_data.as_object_mut() {
        obj.remove("ac_modal");
        obj.remove("ac_modal_inputs");
        if let Some(ch) = channel {
            obj.insert("_channel".into(), Value::String(ch.to_string()));
        }
    }
    let private_metadata = forward_data.to_string();

    // Build the modal view.
    let title_text = action_data
        .get("routeToCardId")
        .and_then(Value::as_str)
        .unwrap_or("Input Required");
    let view = json!({
        "type": "modal",
        "title": { "type": "plain_text", "text": title_text.chars().take(24).collect::<String>() },
        "submit": { "type": "plain_text", "text": "Submit" },
        "close": { "type": "plain_text", "text": "Cancel" },
        "private_metadata": private_metadata.chars().take(3000).collect::<String>(),
        "blocks": modal_blocks,
    });

    let api_body = json!({
        "trigger_id": trigger_id,
        "view": view,
    });

    // Call views.open API.
    let token = match get_secret_string(DEFAULT_BOT_TOKEN_KEY) {
        Ok(t) => t,
        Err(err) => {
            return http_out_error(500, &format!("cannot open modal: secret error: {err}"));
        }
    };
    let api_url = format!("{}/views.open", DEFAULT_API_BASE);
    let req_body = serde_json::to_vec(&api_body).unwrap_or_else(|_| b"{}".to_vec());
    let request = client::Request {
        method: "POST".to_string(),
        url: api_url,
        headers: vec![
            ("Content-Type".into(), "application/json".into()),
            ("Authorization".into(), format!("Bearer {token}")),
        ],
        body: Some(req_body),
    };
    let resp = client::send(&request, None, None);
    if let Err(err) = &resp {
        return http_out_error(500, &format!("views.open transport error: {}", err.message));
    }
    let resp = resp.unwrap();
    if resp.status < 200 || resp.status >= 300 {
        return http_out_error(500, &format!("views.open returned status {}", resp.status));
    }

    // Return 200 with no events — the modal submission will arrive later.
    let out = HttpOutV1 {
        status: 200,
        headers: Vec::new(),
        body_b64: STANDARD.encode(b""),
        events: vec![],
    };
    http_out_v1_bytes(&out)
}

/// Handle `view_submission` — user submitted a Slack modal that was opened
/// from an AC Input.Text button. Extract input values, merge with the
/// original action data (from private_metadata), and create an envelope.
///
/// `verified` is true only when the provider authenticated the request itself;
/// it alone allows a caller block to be stamped.
pub(super) fn handle_view_submission(submission: &Value, verified: bool) -> Vec<u8> {
    let user = submission
        .get("user")
        .and_then(|v| v.get("id"))
        .and_then(Value::as_str)
        .map(|s| s.to_string());

    // Parse the private_metadata to recover the original action data.
    let private_metadata = submission
        .get("view")
        .and_then(|v| v.get("private_metadata"))
        .and_then(Value::as_str)
        .unwrap_or("{}");
    let mut action_data: Value = serde_json::from_str(private_metadata).unwrap_or(json!({}));

    // Extract channel from preserved metadata.
    let channel = action_data
        .get("_channel")
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    if let Some(obj) = action_data.as_object_mut() {
        obj.remove("_channel");
    }

    // Extract input values from the modal state.
    let state_values = submission
        .get("view")
        .and_then(|v| v.get("state"))
        .and_then(|v| v.get("values"))
        .cloned()
        .unwrap_or(json!({}));

    // Flatten: state.values is { block_id: { action_id: { type, value/selected_option } } }
    let mut input_values: BTreeMap<String, String> = BTreeMap::new();
    if let Some(blocks) = state_values.as_object() {
        for (_block_id, actions) in blocks {
            if let Some(actions_obj) = actions.as_object() {
                for (action_id, action_val) in actions_obj {
                    // plain_text_input → "value"
                    // static_select → "selected_option.value"
                    // multi_static_select → "selected_options[].value"
                    let value = if let Some(v) = action_val.get("value").and_then(Value::as_str) {
                        v.to_string()
                    } else if let Some(opt) = action_val.get("selected_option") {
                        opt.get("value")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string()
                    } else if let Some(opts) =
                        action_val.get("selected_options").and_then(Value::as_array)
                    {
                        opts.iter()
                            .filter_map(|o| o.get("value").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                            .join(",")
                    } else {
                        String::new()
                    };
                    if !value.is_empty() {
                        input_values.insert(action_id.clone(), value);
                    }
                }
            }
        }
    }

    // Merge input values into action_data.
    if let Some(obj) = action_data.as_object_mut() {
        for (k, v) in &input_values {
            obj.insert(k.clone(), Value::String(v.clone()));
        }
    }

    // Build the action text for the envelope.
    let route_to_card = action_data
        .get("routeToCardId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let action_text = if !route_to_card.is_empty() {
        format!("[card:{route_to_card}]")
    } else {
        "[modal:submit]".to_string()
    };

    let caller = verified
        .then(|| slack_auth_core::caller_for_view_submission(submission, channel.as_deref()))
        .flatten();
    let mut envelope = build_slack_envelope(action_text, channel.clone(), user);
    set_verified_caller(&mut envelope, caller);
    // Forward all action data fields to metadata.
    if let Some(obj) = action_data.as_object() {
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
        .insert("slack.modal_submission".into(), "true".to_string());
    // Also insert raw input values for easy access.
    for (k, v) in &input_values {
        envelope.metadata.insert(format!("input.{k}"), v.clone());
    }

    // Return empty response body to close the modal (Slack requires empty or
    // `{"response_action":"clear"}` to dismiss). Events are processed by the operator.
    let out = HttpOutV1 {
        status: 200,
        headers: Vec::new(),
        body_b64: STANDARD.encode(b""),
        events: vec![envelope],
    };
    http_out_v1_bytes(&out)
}
