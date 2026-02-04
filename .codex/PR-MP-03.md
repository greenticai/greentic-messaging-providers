---
id: PR-03
track: providers
depends_on: [PR-02]
---
# PR-03: Update greentic-messaging-tester to use common planned-mode encode engine

## Goal
Remove duplicated planned-mode encoding logic from `greentic-messaging-tester` by delegating payload construction to the shared encode engine crate introduced in PR-02.

## Changes
- In `greentic-messaging-tester` planned-mode path:
  - Call provider `render_plan` as before
  - Replace any direct provider `encode` call / local encoding logic with:
    - `encode_from_render_plan(...)` from the shared crate
  - Call provider `send_payload` with the returned `ProviderPayload`
- Ensure debug output remains helpful but does not include secret values:
  - print plan/output JSON safely
  - keep `body_b64` printing optional behind debug flag
- Add/adjust tests:
  - one integration-style test that runs the planned pipeline:
    - render_plan (WASM) → common encode → send_payload (WASM with mock transport)
  - validate stable output shapes and no panics

## Acceptance criteria
- Tester planned mode uses the shared encode engine
- No provider changes
- Tests pass and CI stays green
