# PR-01 / PR-02 Implementation Checklist

Source of truth: decisions provided in chat on 2026-02-16.

## Status Legend
- [ ] not started
- [~] in progress
- [x] complete

## P0 Cross-cutting (blocks correctness)

- [x] Deterministic key helpers in shared crate (`crates/provider-common`)
  - [x] `providers:messaging:{provider_id}:tenants:{tenant_id}:config`
  - [x] `providers:messaging:{provider_id}:tenants:{tenant_id}:provenance`
  - [x] `providers:messaging:{provider_id}:tenants:{tenant_id}:state:{state_name}`
  - [x] Team-scoped variants with `teams:{team_id}` inserted between tenant and key suffix.
  - [x] Unit tests covering with/without team.

- [x] Provenance model + canonical persistence helpers
  - [x] Struct includes `describe_hash`, `artifact_digest`, `schema_hash`.
  - [x] Canonical CBOR helpers used for persisted config/provenance/state.
  - [x] No persisted artifact uses `serde_cbor::to_vec`.

- [x] Remove contract implementation + diagnostics
  - [x] Mandatory delete config key(s). (registry fixture orchestration smoke)
  - [x] Mandatory delete provenance key(s). (registry fixture orchestration smoke)
  - [x] Mandatory delete provider-owned state key namespace. (registry fixture orchestration smoke)
  - [x] Best-effort webhook/subscription/token cleanup if capabilities granted. (remove cleanup steps + orchestration smoke capability gating + execution assertions)
  - [x] Best-effort provider-owned secret cleanup. (remove cleanup steps + orchestration smoke secret cleanup execution assertions)
  - [x] If capability missing: return clear skipped diagnostics while succeeding for accessible keys. (registry fixture orchestration smoke)

## P0 Provider lifecycle semantics

- [x] `messaging-provider-slack`
  - [x] `default`: ask only required missing/empty
  - [x] `setup`: ask all setup options
  - [x] `upgrade`(update): merge from existing config, preserve unspecified
  - [x] `remove`: deterministic cleanup plan/empty config semantics
  - [x] strict validation in `apply-answers`

- [x] `messaging-provider-telegram`
- [x] `messaging-provider-webex`
- [x] `messaging-provider-teams`
- [x] `messaging-provider-email`
- [x] `messaging-provider-webchat`
- [x] `messaging-provider-whatsapp`
- [x] `messaging-provider-dummy`

## P1 Defense-in-depth validation + i18n

- [x] Orchestration re-validation against `describe.config_schema`. (registry fixture orchestration smoke)
- [x] Real English i18n content for QA/UI labels in all providers (no placeholders/key-echo).

## P2 Tests / CI strategy

- [x] Shared E2E lifecycle test path: setup(default) -> update(upgrade) -> remove. (registry fixture orchestration smoke)
- [x] Negative tests: strict schema validation failures. (registry fixture negative validation smoke)
- [x] Negative tests: missing required secret prompts. (registry fixture missing secret prompt smoke)
- [x] Nightly real-provider tests: `telegram`, `slack`, `webchat` initial scope.
- [x] Nightly behavior:
  - [x] skip automatically when env vars are missing
  - [x] retry once
  - [x] non-blocking from mainline PR CI
  - [x] quarantine via `NIGHTLY_PROVIDERS` env filter

## Migration behavior (PR notes + code)

- [x] Detect legacy/stub-era keys.
- [x] Best-effort read/convert on upgrade path when enough data exists.
- [x] Otherwise prompt in upgrade path, then write canonical keys.
- [x] Document behavior in PR notes.
