# Provider setup/diagnostics flows

Branch: `feat/provider-setup-flows-all` (creation blocked in workspace; changes staged here).

Providers and lifecycle flows added to every pack:

- messaging.slack.api → flows: setup_default, setup_custom, diagnostics, verify_webhooks, rotate_credentials
- messaging.telegram.bot → flows: setup_default, setup_custom, diagnostics, verify_webhooks
- messaging.email.smtp → flows: setup_default, setup_custom, diagnostics
- messaging.teams.graph → flows: setup_default, setup_custom, diagnostics, verify_webhooks
- messaging.webchat → flows: setup_default, setup_custom, diagnostics, verify_webhooks
- messaging.webex.bot → flows: setup_default, setup_custom, diagnostics, verify_webhooks
- messaging.whatsapp.cloud → flows: setup_default, setup_custom, diagnostics, verify_webhooks
- messaging.dummy → flows: setup_default, setup_custom, diagnostics

Packaging updates:
- Each pack now defines flows under `flows/` and lists them in `pack.yaml`/`pack.manifest.json`.
- Added `extensions.messaging.provider_flow_hints` (canonical kind) per pack pointing provider ids to lifecycle flows.
- `tools/generate_pack_metadata.py` copies flows from `pack.yaml`, normalizes provider extension keys, and keeps flow hints.
- `tools/publish_packs_oci.sh` rebuilds packs (dry-run friendly) and `tools/validate_pack_extensions.py` ensures provider extension id correctness.

Artifacts:
- `./tools/sync_packs.sh` + `DRY_RUN=1 ./tools/publish_packs_oci.sh` refreshed `dist/packs/*.gtpack` and `packs.lock.json` with flow assets included.
