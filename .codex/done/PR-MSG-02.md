# PR-MSG-02 — greentic-messaging-providers: provider packs/fixtures for no-network E2E with flow/pack/operator

Repo: `greentic-messaging-providers` (and greentic-integration if used)

## Goal
Provide deterministic fixtures and offline E2E validation for provider components:
- fixture registry entries for providers (describe/qa/apply outputs)
- compatibility checks with greentic-flow wizard and greentic-pack doctor (offline)
- CI job ensuring no regressions

## Decisions locked (2026-02-11)
- Target ABI: **greentic:component@0.6.0** world `component-v0-v6-v0`.
- Scope decision: fixtures are required for **all provider components** in this repo (matching PR-MSG-01 all-provider migration).
- Contract authority: **WASM `describe()`** is source of truth (operations + inline SchemaIR + config_schema).
- Validation: strict by default (no silent accept). Any escape hatches must be explicit flags.
- Encodings: CBOR everywhere; use canonical CBOR encoding for stable hashing and deterministic artifacts.
- Hashes:
  - `describe_hash = sha256(canonical_cbor(typed_describe))`
  - `schema_hash = sha256(canonical_cbor({input, output, config}))` recomputed from typed SchemaIR values.
- i18n: `component-i18n.i18n-keys()` required for 0.6.0 components; QA specs must reference only known keys.


## Scope
### In-scope
- Add `tests/fixtures/registry/` aligned with greentic-flow/pack format.
- Provide lightweight fixtures for every provider, including at least:
  - `describe.cbor`
  - `qa_setup.cbor` (and optionally `qa_default`/`qa_upgrade`/`qa_remove` if trivial)
  - `apply_setup_config.cbor`
- Fixture policy:
  - commit frozen `.cbor` bytes
  - include regeneration script for maintainers
  - CI verifies fixture stability (no regeneration in CI)
- Add tests that run:
  - flow add-step/update/remove using fixture resolver (direct handler call)
  - pack doctor strict using fixture resolver
  - operator setup using fixture resolver
- Add fixture sanity checks for every provider.

### Out-of-scope
- Live API tests
- Performance optimizations

## Implementation tasks
1) Produce fixtures for all providers from actual provider components (or typed structs), then freeze bytes.
2) Add/maintain fixture regeneration script (manual/local use).
3) Add offline fixture tests across flow/pack/operator compatibility paths.
4) Add cross-repo integration tests where appropriate (`greentic-integration`).
5) Add CI checks:
   - `cargo test` (offline)
   - fixture sanity checks for every provider
   - fixture stability (no drift in committed frozen bytes)

## Acceptance criteria
- Offline E2E tests pass.
- Fixtures exist for every provider and are stable/shared-compatible.
- `cargo test` passes.
