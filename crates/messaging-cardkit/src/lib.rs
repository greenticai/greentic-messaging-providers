//! Lightweight MessageCard rendering kit for operator/runner consumers.
//!
//! Provisions a self-contained subset of the `messaging_card` stack without depending
//! on `gsm_core`, using only DTOs from `greentic-types`.

use anyhow::Context;
use serde_json::Value;
use std::sync::Arc;

pub mod messaging_card;
mod profiles;

pub use messaging_card::{
    CapabilityProfile, MessageCard, MessageCardEngine, MessageCardKind, PlatformPreview,
    PlatformRenderer, RenderIntent, RenderResponse, RenderSnapshot, RenderSpec, Tier,
};
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
        let preview = PlatformRenderer::from_snapshot(
            &snapshot,
            Some(profiles.tier(provider_type).unwrap_or_default()),
        );
        Ok(RenderResponse {
            intent: spec.intent,
            payload: snapshot.output.payload.clone(),
            preview,
            warnings: snapshot.output.warnings.clone(),
            downgraded: snapshot.downgraded,
            capability: profiles.capability_profile(provider_type),
        })
    }
}
