# Flow Generation Audit

## Status: Simplified (Capability-Driven)

As of v0.4.34, messaging provider packs no longer use generated flows. The legacy
`ci/gen_flows.sh` → `greentic-messaging-packgen` pipeline has been superseded by
the capability-driven pattern.

## Current State

Each provider pack contains 2 hand-authored flow files:

- `flows/setup_default.ygtc` — single-node flow invoking `messaging.configure`
- `flows/requirements.ygtc` — single-node flow invoking `messaging.configure`

Plus their `*.resolve.json` sidecar files pointing to the local provider WASM.

These flows are trivial (one component node each) and do not require generation.
All QA operations (qa-spec, apply-answers, i18n-keys) are handled natively by the
provider WASM component, invoked directly by the operator.

## Legacy (Removed)

The following flow generation infrastructure is no longer used for messaging packs:

- `ci/gen_flows.sh` — generated flows via `greentic-messaging-packgen`
- `crates/greentic-messaging-packgen/` — flow generator using `greentic-flow` CLI
- Generated `*.ygtc` files: setup_default (multi-node), setup_custom, diagnostics,
  verify_webhooks, rotate_credentials, sync_subscriptions, default, remove, update

The packgen crate still exists in the workspace but is not invoked for the simplified packs.
