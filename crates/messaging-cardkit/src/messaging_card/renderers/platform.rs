use crate::messaging_card::types::{PlatformPreview, RenderSnapshot, Tier};

pub struct PlatformRenderer;

impl PlatformRenderer {
    pub fn from_snapshot(
        snapshot: &RenderSnapshot,
        tier_override: Option<Tier>,
    ) -> PlatformPreview {
        PlatformPreview {
            payload: snapshot.output.payload.clone(),
            tier: tier_override.unwrap_or(snapshot.tier),
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
