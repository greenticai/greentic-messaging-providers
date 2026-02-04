use super::types::{MessageCard, RenderOutput, RenderSnapshot, RenderSpec, Tier};
use anyhow::Result;

#[derive(Clone, Debug, Default)]
pub struct MessageCardEngine;

impl MessageCardEngine {
    pub fn bootstrap() -> Self {
        Self
    }

    pub fn render_spec(&self, card: &MessageCard) -> Result<RenderSpec> {
        Ok(RenderSpec::card(card.clone()))
    }

    pub fn render_snapshot(
        &self,
        _provider_type: &str,
        spec: &RenderSpec,
    ) -> Result<RenderSnapshot> {
        let output = RenderOutput {
            payload: spec.card.as_value().clone(),
            warnings: Vec::new(),
            used_modal: false,
            limit_exceeded: false,
            sanitized_count: 0,
            url_blocked_count: 0,
        };

        Ok(RenderSnapshot {
            tier: Tier::Premium,
            target_tier: Tier::Premium,
            downgraded: false,
            output,
        })
    }
}
