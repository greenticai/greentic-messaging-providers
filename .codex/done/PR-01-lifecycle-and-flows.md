# PR-01: Complete 0.6 lifecycle semantics + replace stub provider setup flows

## Goals
- Complete 0.6 lifecycle semantics for all messaging providers:
  - default: use defaults; only ask required missing/empty items
  - setup: ask all setup options
  - update: start from existing config; ask changes; preserve unspecified
  - remove: remove all provider config/state/secrets as defined by provider
- Replace no-op setup flows (`actions: []`) with real lightweight flows that drive the component self-described QA.
- Remove remaining schema-core bridge wiring in packs/manifests for 0.6 path (component authoritative).
- Persist deterministic config keys + provenance (describe_hash + artifact digest) via runtime/operator hooks that providers call through granted host refs.
- Add E2E tests (mock + optional nightly real).

## Implementation Steps
0) Prep: update to new mode name
   - After greentic-interfaces/types rename lands, update each provider component WIT/bindings accordingly.

1) Provider component mode semantics (P0):
   For each provider component (dummy/email/slack/teams/telegram/webchat/webex/whatsapp):
   - Implement mode dispatch for: default/setup/update/remove
   - Ensure default asks only required missing values:
     - Use existing config + secrets store reads to decide what’s missing.
   - Ensure update merges with existing config and preserves unspecified fields.
   - Ensure remove emits cleanup behavior (delete keys, revoke webhooks, etc.) via granted host refs.

2) Replace stub flows in packs (P0):
   - `packs/messaging-*/flows/setup_default.ygtc` and `setup_custom.ygtc` currently no-op.
   - Implement lightweight flows that:
     a) resolve component (0.6)
     b) call describe + qa-spec(mode)
     c) ask questions (via greentic-qa)
     d) call apply-answers
     e) validate config against component schema (if host/tooling supports)
     f) persist config/provenance deterministically
   - Differentiate:
     - setup_default uses mode=default
     - setup_custom uses mode=setup

3) Remove schema-core bridge from pack.manifest.json/spec integration (P0/P1):
   - Replace `greentic:provider/schema-core@1.0.0` references and schema-core-api assets for 0.6 provider lifecycle.
   - Ensure schema + secret requirements are sourced from component self-description exports.

4) Deterministic config keys + provenance (P0/P1):
   - Define deterministic keys for provider config (tenant + provider id [+ team if applicable]).
   - On write, persist:
     - canonical CBOR config
     - describe_hash + artifact digest + schema_hash
   - Ensure remove deletes same keys.

5) i18n improvements (P1):
   - Replace key-echo bundles with real locale assets (at least en).
   - Ensure QA prompt keys are stable and translated.

6) Tests (P2 with some P1):
   - Add a shared E2E harness in `crates/provider-tests`:
     - setup(default) -> update -> remove
     - validates deterministic persisted state + provenance
   - Add negative tests:
     - schema validation failure
     - missing required secret prompts

7) Run:
   - `cargo fmt`
   - `cargo clippy -D warnings`
   - `cargo test` (workspace + provider-tests)

## Acceptance Criteria
- All providers implement default/setup/update/remove with correct semantics (not setup-only).
- Pack flows are no longer `actions: []` and actually drive lifecycle.
- No 0.6 schema-core bridge remains in messaging provider packs.
- Deterministic config persistence + provenance is present and tested.
- Tests pass.

## Migration Behavior (Legacy/Stub-era Keys)
- Shared legacy key detection is centralized in `crates/provider-common/src/lifecycle_keys.rs`:
  - `legacy_messaging_config_keys(...)`
  - `legacy_messaging_provenance_keys(...)`
- Upgrade path behavior (validated in fixture orchestration smoke):
  1. Read canonical deterministic keys first.
  2. If canonical config key is missing, probe known legacy/stub-era config keys and decode.
  3. If decode succeeds and payload is object-like, write canonical config key in canonical CBOR and remove legacy key.
  4. If legacy payload is invalid or insufficient, emit diagnostics, run upgrade QA prompt path, and write canonical config after answers are applied.
  5. If canonical provenance key is missing, probe legacy provenance keys and migrate valid payloads to canonical provenance key.
- Safety boundary:
  - Migration is best-effort only; no destructive fallback beyond provider-owned key namespace.
  - Failed legacy decode does not block lifecycle; it triggers prompt-based upgrade flow.

