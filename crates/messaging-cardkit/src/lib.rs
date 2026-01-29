//! Lightweight MessageCard rendering kit for operator/runner consumers.
//!
//! Exposes the shared engine/renderers from `gsm_core` without the GSM gateway/egress/NATS plumbing.

use anyhow::Context;
use serde_json::Value;
use std::sync::Arc;

pub use gsm_core::messaging_card::downgrade::CapabilityProfile;
pub use gsm_core::messaging_card::renderers::{
    PlatformRenderer, RenderOutput, SlackRenderer, TeamsRenderer, TelegramRenderer,
    WebChatRenderer, WebexRenderer, WhatsAppRenderer,
};
pub use gsm_core::messaging_card::tier::Tier;
pub use gsm_core::messaging_card::{
    AuthRenderSpec, MessageCard, MessageCardEngine, MessageCardKind, RenderIntent, RenderSnapshot,
    RenderSpec,
};

mod profiles;
pub use profiles::{PackProfiles, ProfileSource, StaticProfiles, StaticProfilesBuilder};

/// CardKit renders a MessageCard for a provider type given a profile source.
pub struct CardKit<P: ProfileSource> {
    engine: MessageCardEngine,
    profiles: Arc<P>,
}

impl<P: ProfileSource> CardKit<P> {
    pub fn new(profiles: Arc<P>) -> Self {
        Self {
            engine: MessageCardEngine::bootstrap(),
            profiles,
        }
    }

    pub fn render(
        &self,
        provider_type: &str,
        message_card_json: &Value,
    ) -> anyhow::Result<RenderResponse> {
        let card: MessageCard = serde_json::from_value(message_card_json.clone())?;
        let spec = self.engine.render_spec(&card)?;
        self.render_with_spec(provider_type, &spec)
    }

    pub fn render_with_spec(
        &self,
        provider_type: &str,
        spec: &RenderSpec,
    ) -> anyhow::Result<RenderResponse> {
        Self::render_with_engine(
            &self.engine,
            provider_type,
            spec,
            Arc::clone(&self.profiles),
        )
    }

    fn render_with_engine(
        engine: &MessageCardEngine,
        provider_type: &str,
        spec: &RenderSpec,
        profiles: Arc<P>,
    ) -> anyhow::Result<RenderResponse> {
        let snapshot = engine
            .render_snapshot(provider_type, spec)
            .context("platform renderer not supported")?;
        let preview = PlatformPreview::from_snapshot(&snapshot);
        Ok(RenderResponse {
            intent: spec.intent(),
            payload: snapshot.output.payload.clone(),
            preview,
            warnings: snapshot.output.warnings.clone(),
            downgraded: snapshot.downgraded,
            capability: profiles.capability_profile(provider_type),
        })
    }
}

#[derive(Clone, Debug)]
pub struct RenderResponse {
    pub intent: RenderIntent,
    pub payload: Value,
    pub preview: PlatformPreview,
    pub warnings: Vec<String>,
    pub downgraded: bool,
    pub capability: Option<CapabilityProfile>,
}

#[derive(Clone, Debug)]
pub struct PlatformPreview {
    pub payload: Value,
    pub tier: Tier,
    pub target_tier: Tier,
    pub downgraded: bool,
    pub used_modal: bool,
    pub limit_exceeded: bool,
    pub sanitized_count: usize,
    pub url_blocked_count: usize,
    pub warnings: Vec<String>,
}

impl PlatformPreview {
    fn from_snapshot(snapshot: &RenderSnapshot) -> Self {
        Self {
            payload: snapshot.output.payload.clone(),
            tier: snapshot.tier,
            target_tier: snapshot.target_tier,
            downgraded: snapshot.downgraded,
            used_modal: snapshot.output.used_modal,
            limit_exceeded: snapshot.output.limit_exceeded,
            sanitized_count: snapshot.output.sanitized_count,
            url_blocked_count: snapshot.output.url_blocked_count,
            warnings: snapshot.output.warnings.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Debug)]
    struct DummyProfiles;

    impl ProfileSource for DummyProfiles {
        fn tier(&self, _: &str) -> Option<Tier> {
            Some(Tier::Premium)
        }
    }

    #[test]
    fn render_for_slack_payload() {
        let profiles = Arc::new(DummyProfiles);
        let kit = CardKit::new(profiles);
        let response = kit
            .render(
                "slack",
                &json!({
                    "kind": "standard",
                    "title": "Test",
                    "text": "Testing render",
                }),
            )
            .expect("renders");
        assert_eq!(response.intent, RenderIntent::Card);
        assert!(response.payload.get("blocks").is_some());
        let expected_profile = CapabilityProfile::for_tier(Tier::Premium);
        assert_eq!(response.capability, Some(expected_profile));
    }
}
