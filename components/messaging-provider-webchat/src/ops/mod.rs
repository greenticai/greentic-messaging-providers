//! WebChat provider operations.
//!
//! The egress pipeline and ingress routing are split into focused submodules so
//! each file stays under the project's 500-line cap and the responsibilities
//! are easy to audit:
//!
//! - `render` — step 1 of the 3-step egress pipeline (`render_plan`). Webchat
//!   is a Tier A (native Adaptive Card) provider.
//! - `encode` — step 2 (`encode_op`): universal → provider payload.
//! - `send_payload` — step 3 (`send_payload`): persist + append bot activity
//!   into the Direct Line conversation state.
//! - `send_typing` — optional typing indicator (ephemeral slot, never history).
//! - `send` — legacy `run`/`send` op (`handle_send`) for direct non-universal
//!   invocation.
//! - `ingest` — inbound HTTP router (`handle_ingest`, `ingest_http`) covering
//!   Direct Line, the `/token` shorthand, and the generic webhook fallback.
//! - `oauth` — `/auth/config` and `/oauth/token-exchange` endpoints.
//! - `envelope` — `ChannelMessageEnvelope` constructors shared by the ingest
//!   paths.
//! - `helpers` — tiny pure `serde_json::Value` extraction helpers.
//!
//! NOTE: `messaging-provider-webchat-gui` re-uses this module via
//! `#[path = "../../messaging-provider-webchat/src/ops/mod.rs"]`. Keep the
//! public surface (`pub(crate) use ...` below) stable for both components.

mod encode;
mod envelope;
mod helpers;
mod identify;
mod ingest;
mod ingest_upload;
mod oauth;
mod render;
mod send;
mod send_payload;
mod send_typing;

pub(crate) use encode::encode_op;
pub(crate) use identify::{IDENTIFY_HINT_JSON, extract_recipient_id};
pub(crate) use ingest::{handle_ingest, ingest_http};
pub(crate) use render::render_plan;
pub(crate) use send::handle_send;
pub(crate) use send_payload::send_payload;
pub(crate) use send_typing::send_typing;
