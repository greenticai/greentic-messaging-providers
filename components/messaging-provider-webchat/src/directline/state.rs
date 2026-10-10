use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::jwt::DirectLineContext;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct StoredActivity {
    pub id: String,
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub from: Option<String>,
    pub timestamp: i64,
    pub watermark: u64,
    #[serde(default)]
    pub raw: Value,
}

/// Storage layout marker. `0` (absent) is the legacy single-blob shape where
/// `activities` holds the whole history; `2` means the value is only a
/// header and each activity lives under its own key (see `activity_log`).
pub const LAYOUT_PER_ACTIVITY: u8 = 2;

/// Conversation header. In the legacy layout it also carried every activity.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ConversationState {
    pub ctx: DirectLineContext,
    pub next_watermark: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub activities: Vec<StoredActivity>,
    #[serde(default)]
    pub flow_binding: Option<String>,
    #[serde(default)]
    pub layout: u8,
    /// Creating token's `sub`; `None` = legacy, reachable only with a token bound to it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_sub: Option<String>,
    /// Only a verified owner can be matched by `sub`; an anonymous `sub` is client-chosen.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub owner_verified: bool,
}

impl ConversationState {
    pub fn new(ctx: DirectLineContext) -> Self {
        ConversationState {
            ctx,
            next_watermark: 0,
            activities: Vec::new(),
            flow_binding: None,
            layout: LAYOUT_PER_ACTIVITY,
            owner_sub: None,
            owner_verified: false,
        }
    }

    pub fn new_owned(ctx: DirectLineContext, sub: &str, verified: bool) -> Self {
        ConversationState {
            owner_sub: Some(sub.to_string()),
            owner_verified: verified,
            ..ConversationState::new(ctx)
        }
    }

    /// True when this value still embeds the activity history (pre-split).
    pub fn is_legacy(&self) -> bool {
        self.layout < LAYOUT_PER_ACTIVITY
    }

    pub fn bump_watermark(&mut self) -> u64 {
        let watermark = self.next_watermark;
        self.next_watermark = self.next_watermark.saturating_add(1);
        watermark
    }
}

pub fn conversation_key(ctx: &DirectLineContext, conversation_id: &str) -> String {
    format!(
        "webchat:conv:{}:{}:{}:{}",
        ctx.env,
        ctx.tenant,
        sanitize_team(ctx.team.as_deref()),
        conversation_id
    )
}

pub fn sanitize_team(team: Option<&str>) -> String {
    team.map(|t| t.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "_".to_string())
}

/// Outlives one host refresh interval (~3.5 s) and lapses quickly once refreshes stop.
pub const TYPING_VISIBLE_MS: i64 = 5_000;

/// Ephemeral typing slot, stored under [`typing_key`] and never in `ConversationState`:
/// the store has no compare-and-swap, so rewriting the conversation could lose a reply.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TypingSlot {
    /// Any bot activity stored at or above this watermark answers the turn.
    pub since_watermark: u64,
    pub until_ms: i64,
}

impl TypingSlot {
    pub fn raise(conversation: &ConversationState, now_ms: i64) -> Self {
        TypingSlot {
            since_watermark: conversation.next_watermark,
            until_ms: now_ms.saturating_add(TYPING_VISIBLE_MS),
        }
    }
}

pub fn typing_key(conv_key: &str) -> String {
    format!("{conv_key}:typing")
}

/// Carries the current `next_watermark` without consuming it, so the WS pump still
/// delivers it; a fresh id per call makes Web Chat re-render the indicator.
pub fn typing_activity(
    slot: &TypingSlot,
    conversation: &ConversationState,
    now_ms: i64,
) -> Option<Value> {
    if now_ms >= slot.until_ms {
        return None;
    }
    let answered = conversation.activities.iter().any(|activity| {
        activity.watermark >= slot.since_watermark && activity.from.as_deref() == Some("bot")
    });
    if answered {
        return None;
    }
    let watermark = conversation.next_watermark;
    let timestamp = chrono::DateTime::from_timestamp_millis(now_ms)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| now_ms.to_string());
    Some(json!({
        "type": "typing",
        "id": format!("typing-{watermark}-{now_ms}"),
        "from": {"id": "bot", "role": "bot"},
        "timestamp": timestamp,
        "watermark": watermark.to_string(),
        "channelData": {"watermark": watermark},
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conversation_with(activities: Vec<(u64, &str)>, next: u64) -> ConversationState {
        let mut state = ConversationState::new(DirectLineContext {
            env: "env".into(),
            tenant: "tenant".into(),
            team: None,
        });
        state.next_watermark = next;
        state.activities = activities
            .into_iter()
            .map(|(wm, from)| StoredActivity {
                id: format!("a-{wm}"),
                type_: "message".into(),
                text: Some("x".into()),
                from: Some(from.into()),
                timestamp: 0,
                watermark: wm,
                raw: Value::Null,
            })
            .collect();
        state
    }

    #[test]
    fn typing_key_is_a_sibling_of_the_conversation_key() {
        assert_eq!(
            typing_key("webchat:conv:e:t:_:c"),
            "webchat:conv:e:t:_:c:typing"
        );
    }

    #[test]
    fn raise_records_the_current_watermark_and_a_short_lifetime() {
        let conv = conversation_with(vec![(0, "alice")], 1);
        let slot = TypingSlot::raise(&conv, 1_000);
        assert_eq!(slot.since_watermark, 1);
        assert_eq!(slot.until_ms, 1_000 + TYPING_VISIBLE_MS);
    }

    #[test]
    fn live_unanswered_slot_synthesizes_one_bot_typing_activity() {
        let conv = conversation_with(vec![(0, "alice")], 1);
        let slot = TypingSlot::raise(&conv, 1_000);
        let activity = typing_activity(&slot, &conv, 2_000).expect("typing shown");
        assert_eq!(activity["type"], "typing");
        assert_eq!(activity["from"]["role"], "bot");
        assert_eq!(activity["watermark"], "1");
        assert_eq!(activity["channelData"]["watermark"], 1);
        assert!(activity.get("text").is_none(), "typing must carry no text");
    }

    #[test]
    fn a_lapsed_slot_shows_nothing() {
        let conv = conversation_with(vec![], 1);
        let slot = TypingSlot::raise(&conv, 1_000);
        assert!(typing_activity(&slot, &conv, 1_000 + TYPING_VISIBLE_MS).is_none());
    }

    #[test]
    fn a_bot_activity_stored_after_the_raise_hides_typing() {
        let before = conversation_with(vec![(0, "alice")], 1);
        let slot = TypingSlot::raise(&before, 1_000);
        let answered = conversation_with(vec![(0, "alice"), (1, "bot")], 2);
        assert!(typing_activity(&slot, &answered, 2_000).is_none());
    }

    #[test]
    fn a_user_message_after_the_raise_does_not_hide_typing() {
        let before = conversation_with(vec![], 1);
        let slot = TypingSlot::raise(&before, 1_000);
        let user_again = conversation_with(vec![(1, "alice")], 2);
        let activity = typing_activity(&slot, &user_again, 2_000).expect("still typing");
        assert_eq!(activity["watermark"], "2");
    }

    #[test]
    fn a_bot_activity_from_before_the_raise_does_not_hide_typing() {
        let conv = conversation_with(vec![(0, "bot"), (1, "alice")], 2);
        let slot = TypingSlot::raise(&conv, 1_000);
        assert!(typing_activity(&slot, &conv, 1_500).is_some());
    }

    #[test]
    fn each_synthesis_gets_a_fresh_id() {
        let conv = conversation_with(vec![], 1);
        let slot = TypingSlot::raise(&conv, 1_000);
        let a = typing_activity(&slot, &conv, 1_100).expect("a");
        let b = typing_activity(&slot, &conv, 1_200).expect("b");
        assert_ne!(a["id"], b["id"]);
    }

    #[test]
    fn conversation_key_includes_parts() {
        let ctx = DirectLineContext {
            env: "env1".into(),
            tenant: "tenant42".into(),
            team: Some("team-X".into()),
        };
        let key = conversation_key(&ctx, "conv-1");
        assert_eq!(key, "webchat:conv:env1:tenant42:team-X:conv-1");
    }

    #[test]
    fn sanitize_team_falls_back() {
        assert_eq!(sanitize_team(Some("  ")), "_");
        assert_eq!(sanitize_team(None), "_");
        assert_eq!(sanitize_team(Some(" team ")), "team");
    }

    #[test]
    fn conversation_state_initial() {
        let ctx = DirectLineContext {
            env: "env".into(),
            tenant: "tenant".into(),
            team: None,
        };
        let mut state = ConversationState::new(ctx);
        assert_eq!(state.next_watermark, 0);
        assert_eq!(state.flow_binding, None);
        let first = state.bump_watermark();
        assert_eq!(first, 0);
        assert_eq!(state.next_watermark, 1);
    }

    const LEGACY_HEADER: &str =
        r#"{"ctx":{"env":"e","tenant":"t","team":null},"next_watermark":3,"layout":2}"#;

    #[test]
    fn a_header_without_owner_fields_reads_as_legacy() {
        let state: ConversationState = serde_json::from_str(LEGACY_HEADER).expect("legacy header");
        assert_eq!(state.owner_sub, None);
        assert!(!state.owner_verified);
    }

    #[test]
    fn a_legacy_header_round_trips_byte_identically() {
        let state: ConversationState = serde_json::from_str(LEGACY_HEADER).expect("legacy header");
        let bytes = serde_json::to_vec(&state).expect("serialize");
        assert_eq!(
            bytes,
            br#"{"ctx":{"env":"e","tenant":"t","team":null},"next_watermark":3,"flow_binding":null,"layout":2}"#
        );
    }

    #[test]
    fn an_owned_header_round_trips() {
        let ctx = DirectLineContext {
            env: "e".into(),
            tenant: "t".into(),
            team: None,
        };
        let state = ConversationState::new_owned(ctx, "acme:users:7", true);
        let json = serde_json::to_string(&state).expect("serialize");
        assert!(
            json.contains(r#""owner_sub":"acme:users:7","owner_verified":true"#),
            "{json}"
        );
        let back: ConversationState = serde_json::from_str(&json).expect("parse");
        assert_eq!(back, state);
    }

    #[test]
    fn an_anonymous_owner_omits_the_verified_flag() {
        let ctx = DirectLineContext {
            env: "e".into(),
            tenant: "t".into(),
            team: None,
        };
        let state = ConversationState::new_owned(ctx, "guest-1", false);
        let json = serde_json::to_string(&state).expect("serialize");
        assert!(json.contains(r#""owner_sub":"guest-1""#), "{json}");
        assert!(!json.contains("owner_verified"), "{json}");
    }

    #[test]
    fn old_persisted_state_without_flow_binding_deserializes() {
        // State persisted by the previous version lacks `flow_binding`.
        // `#[serde(default)]` must make this deserialize without error.
        let json = r#"{
            "ctx": {"env": "prod", "tenant": "acme", "team": null},
            "next_watermark": 5,
            "activities": []
        }"#;
        let state: ConversationState =
            serde_json::from_str(json).expect("old state must deserialize");
        assert_eq!(state.flow_binding, None);
        assert_eq!(state.next_watermark, 5);
    }

    #[test]
    fn state_with_flow_binding_round_trips() {
        let ctx = DirectLineContext {
            env: "env".into(),
            tenant: "tenant".into(),
            team: None,
        };
        let mut state = ConversationState::new(ctx);
        state.flow_binding = Some("welcome-flow".into());
        let serialized = serde_json::to_string(&state).expect("serialize");
        let deserialized: ConversationState =
            serde_json::from_str(&serialized).expect("deserialize");
        assert_eq!(deserialized.flow_binding, Some("welcome-flow".into()));
    }
}
