//! Telegram provider operations.
//!
//! This module is split across several submodules to keep each file under the
//! project-wide 500 LOC limit while preserving the original behaviour:
//!
//! - [`render`]   — `render_plan` op (TierD planner) with the host-side panic guard.
//! - [`encode`]   — `encode` op: Adaptive Card → Telegram HTML + metadata mapping.
//! - [`send`]     — `send`/`run` op: outbound Telegram message fan-out.
//! - [`send_payload`] — `send_payload` op, `handle_reply`, and the send-payload bridge.
//! - [`ingest`]   — `ingest_http` op: webhook → `ChannelMessageEnvelope` normalization.
//! - [`caller`]   — the verified-caller block `ingest_http` may stamp (host-marker gated).
//! - [`typing`]   — `send_typing` op: `sendChatAction` typing indicator.
//! - [`webhook`]  — `setup_webhook` op: Telegram `setWebhook` invocation.
//! - [`http`]     — thin wrappers around the HTTP client for Telegram API calls.
//! - [`ac_to_html`] / [`ac_inputs`] / [`ac_helpers`] — Adaptive Card → HTML + inline
//!   keyboard converter, split into dispatch / input-element handling / shared
//!   helpers so that each file stays within the 500 LOC budget.

pub(crate) mod ac_helpers;
pub(crate) mod ac_inputs;
pub(crate) mod ac_to_html;
pub(crate) mod caller;
pub(crate) mod encode;
pub(crate) mod form_reply;
pub(crate) mod http;
pub(crate) mod identify;
pub(crate) mod ingest;
pub(crate) mod render;
pub(crate) mod send;
pub(crate) mod send_payload;
pub(crate) mod typing;
pub(crate) mod webhook;

pub(crate) use encode::encode_op;
pub(crate) use identify::{IDENTIFY_HINT_JSON, extract_secret_token};
pub(crate) use ingest::ingest_http;
pub(crate) use render::render_plan;
pub(crate) use send::handle_send;
pub(crate) use send_payload::{handle_reply, send_payload};
pub(crate) use typing::send_typing;
pub(crate) use webhook::setup_webhook;
