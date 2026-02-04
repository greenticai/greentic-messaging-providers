---
id: PR-04
track: providers
depends_on: [PR-03]
---
# PR-04: Update greentic-operator demo send to use common planned-mode encode engine

## Goal
Ensure `greentic-operator` demo send uses the exact same planned-mode pipeline as `greentic-messaging-tester`:
render_plan (provider WASM) → common encode engine → send_payload (provider WASM).
This eliminates drift and keeps pass-through behavior consistent.

## Changes
- In operator demo send path:
  - Call provider `render_plan` as before
  - Replace any local encoding / provider `encode` call with shared `encode_from_render_plan(...)`
  - Call provider `send_payload` with returned `ProviderPayload`
- Keep existing secret error enrichment and missing-URI gathering logic intact (host-side), but make sure it runs against:
  - render_plan output
  - common encode output
  - send_payload output
- Add/adjust smoke tests:
  - demo-bundle run that exercises Webex/WebChat planned send
  - validates that pass-through payloads succeed when secrets exist

## Acceptance criteria
- Operator demo send uses the shared encode engine
- Behavior matches tester for planned mode
- CI stays green
