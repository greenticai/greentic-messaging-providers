# PR-MSG-01 — greentic-messaging-providers: migrate all provider components to component@0.6.0 self-description (CBOR + QA + i18n)

Repo: `greentic-messaging-providers` (and any provider-common crate)

## Goal
Ensure all messaging provider components currently in this repo are true 0.6.0 components:
- Export `greentic:component@0.6.0` world
- `describe()` includes operations + inline SchemaIR + config_schema
- QA modes implemented (default/setup/upgrade/remove)
- i18n keys exported and used (no raw strings)
- CBOR I/O throughout

This makes providers compatible with greentic-flow/pack/operator wizards.

## Decisions locked (2026-02-11)
- Target ABI: **greentic:component@0.6.0** world `component-v0-v6-v0`.
- Scope decision: migrate **every provider component crate currently in `greentic-messaging-providers`** in this PR.
- Contract authority: **WASM `describe()`** is source of truth (operations + inline SchemaIR + config_schema).
- Validation: strict by default (no silent accept). Any escape hatches must be explicit flags.
- Encodings: CBOR everywhere; use canonical CBOR encoding for stable hashing and deterministic artifacts.
- Hashes:
  - `describe_hash = sha256(canonical_cbor(typed_describe))`
  - `schema_hash = sha256(canonical_cbor({input, output, config}))` recomputed from typed SchemaIR values.
- i18n: `component-i18n.i18n-keys()` required for 0.6.0 components; QA specs must reference only known keys.

## Provider migration checklist (auto-discovered)
All crates under `components/` that build a provider component must be migrated.

- [x] `components/messaging-provider-dummy`
- [x] `components/messaging-provider-email`
- [x] `components/messaging-provider-slack`
- [x] `components/messaging-provider-teams`
- [x] `components/messaging-provider-telegram`
- [x] `components/messaging-provider-webchat`
- [x] `components/messaging-provider-webex`
- [x] `components/messaging-provider-whatsapp`
- [x] `components/slack`
- [x] `components/teams`
- [x] `components/telegram`
- [x] `components/webchat`
- [x] `components/webex`
- [x] `components/whatsapp`

## Scope
### In-scope
- Update each provider component crate with one identical migration contract:
  - export `greentic:component@0.6.0` world `component-v0-v6-v0`
  - `describe()` authoritative with:
    - `operations[]` containing at least `run` (plus provider-specific ops)
    - inline SchemaIR for input/output
    - component `config_schema`
    - path-based redaction rules for secret fields
    - deterministic `schema_hash`
  - QA modes: `default/setup/upgrade/remove`
  - i18n: `i18n-keys()` present; QA specs use `I18nText` only
  - CBOR-only I/O with canonical encoding
- Create/extend shared `provider-common` helpers once, then reuse across providers:
  - SchemaIR builders for common secret/token/url patterns
  - canonical CBOR encode helpers
  - schema hash helper
  - QA spec builders with i18n key generation conventions
- Enforce minimal config schema fields per provider:
  - `enabled: bool` (required)
  - required secrets/tokens/webhook secrets (when applicable)
  - required endpoints/URLs (when applicable)
  - path-based redaction rules for secret paths
- Add workspace tests that iterate all provider components and verify:
  - `describe()` decodes
  - operations are non-empty
  - `schema_hash` is correct
  - `i18n-keys()` exists and covers QA keys
- Add per-provider no-network tests for:
  - hash stability
  - `describe` strict-rule validation

### Out-of-scope
- Pack wizard changes (handled in greentic-pack)
- Distribution resolution logic

## Implementation tasks
1) Implement shared helpers in provider-common once (SchemaIR, canonical CBOR, hash, QA+i18n builders).
2) Migrate every provider crate in the checklist to the unified 0.6.0 contract.
3) Add/normalize config schema and redaction rules per provider.
4) Implement QA modes and i18n key coverage across all providers.
5) Add workspace-wide validation tests plus per-provider strict no-network tests.

## Acceptance criteria
- Every provider crate in the checklist builds as a 0.6.0 component.
- Every provider `describe()` is self-describing and passes strict schema rules.
- QA + i18n surfaces are present and consistent for every provider.
- Workspace test confirms schema hash, operations, and i18n QA key coverage for all providers.
- `cargo test` passes.
