//! Adaptive Card → Telegram shared helpers.
//!
//! Contains:
//! - HTML escaping and truncation utilities used by the converter.
//! - `collect_actions` / `collect_select_action` / `label_from_items` /
//!   `collect_text_blocks` / `compact_callback_data` — AC action harvesting.
//! - `build_inline_keyboard_from_metadata` — turns `ac_actions` metadata into a
//!   Telegram inline-keyboard layout (≤ 8 rows × 3 buttons).
//! - `has_pending_text_inputs` / `first_input_placeholder` — inspect the
//!   `ac_pending_inputs` metadata written by `encode_op` to drive ForceReply.

use serde_json::{Value, json};

use super::ac_to_html::{AcInput, AcInputKind};

fn debug_enabled() -> bool {
    matches!(
        std::env::var("TELEGRAM_DEBUG")
            .or_else(|_| std::env::var("GREENTIC_DEBUG"))
            .as_deref(),
        Ok("1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON")
    )
}

macro_rules! telegram_debug {
    ($($arg:tt)*) => {
        if debug_enabled() {
            provider_common::telemetry::log(
                provider_common::telemetry::Level::Debug,
                &format!($($arg)*),
                &[provider_common::telemetry::Field {
                    key: provider_common::telemetry::field::PROVIDER,
                    value: "telegram",
                }],
            );
        }
    };
}

/// Collect AC actions into a flat JSON array for inline keyboard.
pub(crate) fn collect_actions(action_list: &[Value], actions: &mut Vec<Value>) {
    for action in action_list {
        let atype = action
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let title = action
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if title.is_empty() {
            continue;
        }
        match atype {
            "Action.OpenUrl" => {
                let url = action.get("url").and_then(Value::as_str).unwrap_or("");
                actions.push(json!({"title": title, "url": url}));
            }
            "Action.Submit" | "Action.Execute" => {
                let mut btn = json!({"title": title});
                if let Some(data) = action.get("data") {
                    btn.as_object_mut()
                        .unwrap()
                        .insert("data".into(), data.clone());
                }
                actions.push(btn);
            }
            _ => {
                // Action.ShowCard, Action.ToggleVisibility — no Telegram equivalent.
                // Store as callback button so user at least sees the label.
                actions.push(json!({"title": title}));
            }
        }
    }
}

/// Check if metadata has pending text inputs (Input.Text that needs user reply).
pub(crate) fn has_pending_text_inputs(metadata: &greentic_types::MessageMetadata) -> bool {
    metadata
        .get("ac_pending_inputs")
        .and_then(|s| serde_json::from_str::<Vec<AcInput>>(s).ok())
        .is_some_and(|inputs| inputs.iter().any(|i| matches!(i.kind, AcInputKind::Text)))
}

/// Get placeholder text for first pending text input.
pub(crate) fn first_input_placeholder(metadata: &greentic_types::MessageMetadata) -> String {
    metadata
        .get("ac_pending_inputs")
        .and_then(|s| serde_json::from_str::<Vec<AcInput>>(s).ok())
        .and_then(|inputs| {
            inputs
                .iter()
                .find(|i| matches!(i.kind, AcInputKind::Text))
                .map(|i| {
                    if !i.placeholder.is_empty() {
                        i.placeholder.clone()
                    } else {
                        i.label.clone()
                    }
                })
        })
        .unwrap_or_default()
}

/// Extract a human-readable label from a Container/Column's child items.
///
/// Searches recursively into ColumnSet/Column/Container children so that
/// labels are found even when TextBlocks are nested (e.g. Container →
/// ColumnSet → Column → TextBlock).
pub(crate) fn label_from_items(items: &[Value]) -> String {
    // Collect all TextBlocks by flattening nested structures.
    let mut text_blocks: Vec<&Value> = Vec::new();
    collect_text_blocks(items, &mut text_blocks);

    // First try: bold TextBlock.
    for tb in &text_blocks {
        if tb
            .get("weight")
            .and_then(Value::as_str)
            .is_some_and(|w| w.eq_ignore_ascii_case("bolder"))
            && let Some(text) = tb.get("text").and_then(Value::as_str)
        {
            let t = text.trim();
            if !t.is_empty() {
                return t.chars().take(64).collect();
            }
        }
    }
    // Fallback: first non-empty TextBlock.
    for tb in &text_blocks {
        if let Some(text) = tb.get("text").and_then(Value::as_str) {
            let t = text.trim();
            if !t.is_empty() {
                return t.chars().take(64).collect();
            }
        }
    }
    String::new()
}

/// Recursively collect all TextBlock elements from nested AC structures.
fn collect_text_blocks<'a>(items: &'a [Value], out: &mut Vec<&'a Value>) {
    for item in items {
        let etype = item.get("type").and_then(Value::as_str).unwrap_or_default();
        match etype {
            "TextBlock" => out.push(item),
            "ColumnSet" => {
                if let Some(cols) = item.get("columns").and_then(Value::as_array) {
                    for col in cols {
                        if let Some(col_items) = col.get("items").and_then(Value::as_array) {
                            collect_text_blocks(col_items, out);
                        }
                    }
                }
            }
            "Column" | "Container" => {
                if let Some(child_items) = item.get("items").and_then(Value::as_array) {
                    collect_text_blocks(child_items, out);
                }
            }
            _ => {}
        }
    }
}

/// If an AC element has a `selectAction` (Action.Submit/Execute), convert it
/// to an inline keyboard button entry using the element's child text as label.
pub(crate) fn collect_select_action(element: &Value, actions: &mut Vec<Value>) {
    let sa = match element.get("selectAction") {
        Some(sa) => sa,
        None => return,
    };
    let atype = sa.get("type").and_then(Value::as_str).unwrap_or_default();
    if atype != "Action.Submit" && atype != "Action.Execute" {
        return;
    }
    let items = element
        .get("items")
        .and_then(Value::as_array)
        .map(|a| a.as_slice())
        .unwrap_or(&[]);
    let label = label_from_items(items);
    if label.is_empty() {
        return;
    }
    let mut btn = json!({ "title": label });
    if let Some(data) = sa.get("data") {
        btn.as_object_mut()
            .unwrap()
            .insert("data".into(), data.clone());
    }
    actions.push(btn);
}

/// Compress AC action data to fit Telegram's 64-byte callback_data limit.
/// Uses abbreviated keys: "r" for routeToCardId, "c" for cardId.
/// The ingest_http callback_query handler recognises both full and abbreviated keys.
pub(crate) fn compact_callback_data(data: &Value) -> Value {
    let mut compact = serde_json::Map::new();
    if let Some(rtc) = data.get("routeToCardId").and_then(Value::as_str) {
        compact.insert("r".into(), Value::String(rtc.to_string()));
    }
    if let Some(cid) = data.get("cardId").and_then(Value::as_str) {
        compact.insert("c".into(), Value::String(cid.to_string()));
    }
    if compact.is_empty() {
        // No routing fields — fall back to full data.
        data.clone()
    } else {
        Value::Object(compact)
    }
}

/// Escape HTML special characters for Telegram's HTML parse mode.
pub(crate) fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Truncate HTML string to at most `max` chars, preserving char boundaries.
pub(crate) fn truncate_html(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let truncated: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{truncated}\u{2026}")
}

/// Build Telegram inline keyboard rows from AC actions stored in metadata.
///
/// Supports multiple rows (max 8 rows, max 3 buttons per row).
/// URL buttons use `url` field, others use `callback_data` (max 64 bytes).
pub(crate) fn build_inline_keyboard_from_metadata(
    metadata: &greentic_types::MessageMetadata,
) -> Vec<Vec<Value>> {
    let actions_json = match metadata.get("ac_actions") {
        Some(s) => s,
        None => return Vec::new(),
    };
    let actions: Vec<Value> = match serde_json::from_str(actions_json) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let max_rows = 8;
    let max_per_row = 3;
    let mut rows: Vec<Vec<Value>> = Vec::new();
    let mut current_row: Vec<Value> = Vec::new();
    for action in &actions {
        let title = action
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let url = action.get("url").and_then(Value::as_str);
        if title.is_empty() {
            continue;
        }
        if current_row.len() >= max_per_row {
            rows.push(current_row);
            current_row = Vec::new();
        }
        if rows.len() >= max_rows {
            break;
        }
        if let Some(url) = url {
            current_row.push(json!({"text": title, "url": url}));
        } else {
            // Use AC action data as callback_data if available (carries routeToCardId, etc.).
            // Telegram callback_data max 64 bytes — compress if needed.
            let has_data = action.get("data").is_some();
            let cb = if let Some(data) = action.get("data") {
                let serialized = data.to_string();
                telegram_debug!(
                    "tg build_keyboard: title={title} data_len={} data={serialized}",
                    serialized.len()
                );
                if serialized.len() <= 64 {
                    serialized
                } else {
                    // Too large — extract only routeToCardId for compact callback_data.
                    let compact = compact_callback_data(data);
                    let compact_str = compact.to_string();
                    telegram_debug!(
                        "tg build_keyboard: compact_len={} compact={compact_str}",
                        compact_str.len()
                    );
                    if compact_str.len() <= 64 {
                        compact_str
                    } else {
                        title.chars().take(64).collect()
                    }
                }
            } else {
                telegram_debug!("tg build_keyboard: title={title} NO data field");
                title.chars().take(64).collect()
            };
            telegram_debug!("tg build_keyboard: final cb={cb} has_data={has_data}");
            current_row.push(json!({"text": title, "callback_data": cb}));
        }
    }
    if !current_row.is_empty() && rows.len() < max_rows {
        rows.push(current_row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use greentic_types::MessageMetadata;

    #[test]
    fn action_collection_keeps_urls_submit_data_and_unknown_labels() {
        let mut actions = Vec::new();
        collect_actions(
            &[
                json!({"type": "Action.OpenUrl", "title": "Open", "url": "https://example.com"}),
                json!({"type": "Action.Submit", "title": "Save", "data": {"cardId": "c1"}}),
                json!({"type": "Action.ToggleVisibility", "title": "More"}),
                json!({"type": "Action.Submit"}),
            ],
            &mut actions,
        );

        assert_eq!(actions.len(), 3);
        assert_eq!(actions[0]["url"], "https://example.com");
        assert_eq!(actions[1]["data"]["cardId"], "c1");
        assert_eq!(actions[2]["title"], "More");
    }

    #[test]
    fn label_from_items_recurses_and_prefers_bolder_text() {
        let items = vec![json!({
            "type": "ColumnSet",
            "columns": [{
                "items": [
                    {"type": "TextBlock", "text": "secondary"},
                    {"type": "TextBlock", "text": "Primary", "weight": "Bolder"}
                ]
            }]
        })];

        assert_eq!(label_from_items(&items), "Primary");
    }

    #[test]
    fn select_action_uses_nested_label_and_carries_data() {
        let element = json!({
            "type": "Container",
            "items": [{"type": "TextBlock", "text": "Choose me"}],
            "selectAction": {"type": "Action.Execute", "data": {"routeToCardId": "next"}}
        });
        let mut actions = Vec::new();

        collect_select_action(&element, &mut actions);

        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0]["title"], "Choose me");
        assert_eq!(actions[0]["data"]["routeToCardId"], "next");
    }

    #[test]
    fn html_escape_and_truncate_preserve_telegram_safety() {
        assert_eq!(html_escape("a < b & c > d"), "a &lt; b &amp; c &gt; d");
        assert_eq!(truncate_html("abcdef", 4), "abc\u{2026}");
        assert_eq!(truncate_html("abc", 4), "abc");
    }

    #[test]
    fn pending_input_metadata_detects_text_and_placeholder() {
        let mut metadata = MessageMetadata::new();
        metadata.insert(
            "ac_pending_inputs".to_string(),
            serde_json::to_string(&vec![
                json!({
                    "id": "choice",
                    "label": "Choice",
                    "placeholder": "",
                    "kind": "choice",
                    "choices": []
                }),
                json!({
                    "id": "comment",
                    "label": "Comment",
                    "placeholder": "Type here",
                    "kind": "text",
                    "choices": []
                }),
            ])
            .unwrap(),
        );

        assert!(has_pending_text_inputs(&metadata));
        assert_eq!(first_input_placeholder(&metadata), "Type here");
    }

    #[test]
    fn inline_keyboard_limits_rows_columns_and_compacts_callback_data() {
        let mut actions = Vec::new();
        for idx in 0..30 {
            actions.push(json!({
                "title": format!("Action {idx}"),
                "data": {
                    "routeToCardId": format!("route-{idx}"),
                    "cardId": "card",
                    "large": "x".repeat(100)
                }
            }));
        }
        let mut metadata = MessageMetadata::new();
        metadata.insert(
            "ac_actions".to_string(),
            serde_json::to_string(&actions).unwrap(),
        );

        let rows = build_inline_keyboard_from_metadata(&metadata);

        assert_eq!(rows.len(), 8);
        assert!(rows.iter().all(|row| row.len() <= 3));
        let callback = rows[0][0]["callback_data"].as_str().unwrap();
        assert!(callback.len() <= 64);
        assert!(callback.contains("\"r\""));
    }
}
