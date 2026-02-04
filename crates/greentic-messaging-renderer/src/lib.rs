//! Canonical renderer for messaging providers.
//! This crate exposes a minimal render plan model plus a no-op renderer that passes
//! Adaptive Card JSON through unchanged.

pub mod context;
pub mod errors;
pub mod mode;
pub mod plan;
pub mod renderer;

pub use context::RenderContext;
pub use errors::RendererError;
pub use mode::RendererMode;
pub use plan::{RenderItem, RenderPlan, RenderTier, RenderWarning};
pub use renderer::{CardRenderer, NoopCardRenderer, render_plan_from_envelope};
