use crate::{
    context::RenderContext,
    mode::RendererMode,
    plan::{RenderItem, RenderPlan, RenderTier},
};
use greentic_types::{ChannelMessageEnvelope, MessageMetadata};
use serde_json::{Value, json};

/// Trait describing a renderer that turns an envelope into a plan.
pub trait CardRenderer {
    fn render_plan(
        &self,
        envelope: &ChannelMessageEnvelope,
        context: &RenderContext,
        mode: RendererMode,
    ) -> RenderPlan;
}

/// No-op renderer that passes text and saved Adaptive Cards through unchanged.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopCardRenderer;

impl CardRenderer for NoopCardRenderer {
    fn render_plan(
        &self,
        envelope: &ChannelMessageEnvelope,
        context: &RenderContext,
        mode: RendererMode,
    ) -> RenderPlan {
        let summary_text = envelope
            .text
            .as_ref()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());

        let mut items = Vec::new();
        if let Some(text) = summary_text.clone() {
            items.push(RenderItem::Text(text));
        }
        if let Some(card) = parse_adaptive_card(&envelope.metadata) {
            items.push(RenderItem::AdaptiveCard(card));
        }

        let debug = json!({
            "mode": format!("{:?}", mode),
            "target": context.target.clone(),
        });

        RenderPlan {
            tier: RenderTier::TierA,
            summary_text,
            items,
            warnings: Vec::new(),
            debug: Some(debug),
        }
    }
}

/// Convenience helper that builds a plan using the no-op renderer.
pub fn render_plan_from_envelope(
    envelope: &ChannelMessageEnvelope,
    context: &RenderContext,
    mode: RendererMode,
) -> RenderPlan {
    NoopCardRenderer.render_plan(envelope, context, mode)
}

fn parse_adaptive_card(metadata: &MessageMetadata) -> Option<Value> {
    metadata
        .get("adaptive_card")
        .and_then(|value| serde_json::from_str::<Value>(value).ok())
}
