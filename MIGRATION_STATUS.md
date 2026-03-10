# Migration Status

- PR-01: Scaffolded workspace with shared crates (`messaging-core`, `provider-common`) and placeholders for components, tools, and CI workflows.
- PR-02: Added secrets-probe component with secrets-store WIT bindings, minimal run logic, manifest, and component build script (falls back to `cargo build` when `cargo-component` resolution fails).
- PR-03: Added Slack provider component (egress/ingress/formatting/refresh stub), WIT bindings for http/secrets/state/logger, secrets manifest, tests, and build integration.
- PR-04: Added CI workflow to run fmt/tests/component builds and upload artifacts.
- PR-05: Added OCI publish workflow and `publish_oci.sh` to push built components and emit `components.lock.json`.
- PR-06: Added Teams provider component (egress/ingress/formatting/refresh stub) with WIT bindings and build integration.
- PR-07: Added Telegram provider component (egress/ingress/formatting/refresh stub) with WIT bindings, manifest, tests, and build integration.
- PR-08: Added WebChat provider component (egress/ingress/formatting/refresh stub) with WIT bindings, manifest, tests, and build integration.
- PR-09: Added Webex provider component (egress/ingress/formatting/refresh stub) with WIT bindings, manifest, tests, and build integration.
- PR-10: Added WhatsApp provider component (egress/ingress/formatting/refresh stub) with WIT bindings, manifest, tests, and build integration.
- PR-11: Added provider conformance tests and docs clarifying build/publish and component coverage.
- PR-12: Added pack publishing flow (gtpack build/push script with dry-run), release workflow for GHCR, CI validation, and README updates on packs/lockfile usage.
- Pack Simplification: Migrated all 8 messaging provider packs to capability-driven pattern (matching `packs/telemetry-otlp/`). Removed legacy generated flows (diagnostics, setup_custom, verify_webhooks, rotate_credentials, sync_subscriptions, etc.), legacy component WASMs (provision, questions, templates, flow-node stubs), and unused extensions (validators, flow_hints). Added `greentic.ext.capabilities.v1` extension to all packs. Each pack now has 1-2 components and 2 flows (setup_default + requirements).
