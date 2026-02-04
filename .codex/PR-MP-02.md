---
id: PR-02
track: providers
depends_on: [PR-01]
---
# PR-02: Add common planned-mode encode engine (host-side) + shared payload types (pass-through)

## Goal
Create a single reusable “planned-mode encode engine” used by both `greentic-messaging-tester` and `greentic-operator` that builds a provider `ProviderPayload` from a provider’s `render_plan` output. For now it is strictly pass-through (no downsampling), but it preserves the seam for future downgrade logic.

## Scope / Key decision
- **Do NOT modify any provider WASM components** in this PR.
- Providers continue to export `render_plan` and `send_payload` as they do today.
- The host (tester/operator) will call:
  1) provider `render_plan` (WASM)
  2) **common encode engine** (Rust crate, pass-through)
  3) provider `send_payload` (WASM)

## Changes
- Add or extend a shared crate/module inside `greentic-messaging-providers` (reuse existing work if already started):
  - Suggested crate name: `greentic-messaging-planned` or `greentic-messaging-encode-engine`
- Define (or reuse if already implemented) shared types used by host side:
  - `ProviderPayload { content_type, body_b64, metadata }`
  - `EncodeResult { ok, payload, warnings, error }`
  - `RenderWarning { code, message, details? }`
- Implement encode engine function (pass-through only):
  - `encode_from_render_plan(render_plan_out_json, message_envelope, provider_hint) -> EncodeResult`
  - Behavior:
    - If render_plan output contains a card/body blob, return it unchanged as payload body (base64 on wire).
    - Else generate Tier-D text payload from message text.
    - If render_plan tier is A/B/C and we’re pass-through-only:
      - emit warning `passthrough_no_downsample`
- Add tests:
  - serde round-trip + stable JSON shape tests for `ProviderPayload` and `EncodeResult`
  - pass-through test: given a render_plan output containing a card JSON/body, decoded `body_b64` matches the input bytes
  - Tier-D test: given text-only, output is `text/plain` (or minimal JSON wrapper) and decodes correctly

## Acceptance criteria
- Shared crate builds and tests pass
- JSON output shapes are stable
- No provider components are changed
- CI stays green

## Notes
- Downsampling/downgrade is explicitly deferred. This PR establishes the common encode seam.
