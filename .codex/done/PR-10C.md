# PR-10C.md (greentic-messaging-providers)
# Title: Add provider-core dummy messaging provider component + pack fixture (CI deterministic)

## Goal
Create a deterministic provider-core messaging provider to unblock runner/deployer/store testing
and to avoid half-migration.

This provider does NOT contact external services; it simulates sends.

## Deliverables
1) WASM component (wasm32-wasip2) implementing provider-core:
- crate: `components/messaging-provider-dummy/`
- provider_type: `messaging.dummy`
- ops: `send`, optional `reply`
Behavior:
- invoke("send", input_json) returns output_json containing:
  - message_id = stable uuid (or hash of input)
  - provider_message_id = "dummy:<hash>"
  - status = "sent"
- validate-config accepts any config that parses

2) Pack fixture:
- `packs/messaging-dummy.gtpack/` (or your pack format)
Includes:
- schemas:
  - config schema: `schemas/messaging/dummy/config.schema.json`
- extension:
  - `extensions["greentic.ext.provider"].inline.providers[0]` with runtime.world pinned to provider-core v1
- component artifact reference to the built WASM

3) Tests
- Build test for the WASM component
- Pack validation test (extension + schemas exist)
- Smoke test that instantiates the component and calls invoke("send")

## Acceptance criteria
- CI can run messaging provider-core flows without network.
- This becomes the baseline fixture for runner PR-08 and integration PR-14.
