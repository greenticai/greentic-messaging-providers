0) Global rule for all repos (tell Codex this every time)

Use this paragraph at the top of every prompt:

Global policy:
- `greentic:component@0.6.0` canonical WIT lives only in `greentic-interfaces`.
- No other repo should define or vendor `package greentic:component@0.6.0` + `world component-v0-v6-v0` in its own `wit/`.
- Any WIT declaring `package greentic:component@0.4.0` or `package greentic:component@0.5.0` is forbidden everywhere.

1) Updated policy

Canonical source of truth
- `greentic:component@0.6.0` canonical WIT exists only in `greentic-interfaces`.

No legacy component packages remain
- Any `.wit` declaring `package greentic:component@0.4.0` is not allowed.
- Any `.wit` declaring `package greentic:component@0.5.0` is not allowed.
- This applies everywhere (including `greentic-interfaces`) unless there is an explicit, temporary, time-bounded compatibility folder (avoid this).

No local redefinitions of canonical world
- Outside `greentic-interfaces`, no `.wit` may define both:
  - `package greentic:component@0.6.0`
  - `world component-v0-v6-v0`

Providers should not compile local component WIT
- Providers should consume canonical WIT via `greentic-interfaces`.
- Guest exports should use `greentic_interfaces_guest::export_component_v060!(...)`.

2) CI/test guard rules (hard fail)

Run in every repo (providers + runner + any repo with WIT).

Guard A — ban legacy component packages
- Fail if any `.wit` contains:
  - `package greentic:component@0.4.0`
  - `package greentic:component@0.5.0`
- No path exceptions.

Guard B — ban duplicated canonical world
- Fail if any `.wit` contains both:
  - `package greentic:component@0.6.0`
  - `world component-v0-v6-v0`
- Exception: allowed only inside the canonical `greentic-interfaces` repo.

Invariant
- `0.4/0.5`: nowhere.
- Canonical `0.6` world: only in `greentic-interfaces`.
- Everyone else: reference/use, never redefine.

3) Migration plan when 0.4.0/0.5.0 are found

1. Inventory
- Locate every `.wit` with `package greentic:component@0.4.0` or `@0.5.0`.
- Classify each occurrence:
  - local legacy definition (must be removed)
  - used by guest exports/bindings (must migrate)

2. Replace with canonical `0.6.0` usage
- Guest components (providers/fixtures):
  - replace old export macros/bindgen targets with `greentic_interfaces_guest::export_component_v060!(...)`
  - ensure 0.6 imports/exports/types consistently
- Host/runtime loading:
  - stop generating bindings from local component WIT
  - validate/load against 0.6 contract metadata from `greentic-interfaces`

3. Remove legacy WIT files
- After references migrate, delete all `component@0.4.0` / `component@0.5.0` WIT files.

4) Clarifications replacing earlier open questions

- “Should `package.wit` (0.5.0) be centralized?”
  - No. Delete/migrate it; no 0.5.0 should remain.

- “Exception list?”
  - Content-based only. Only canonical 0.6 world in `greentic-interfaces`.
  - No exceptions for 0.4/0.5.

- “Hard failure vs auto-sync?”
  - Hard failure.

- “Commit generated `bindings.rs`?”
  - Prefer not committed; generation must target 0.6.

- “Dependency source?”
  - Use crates.io for CI/releases.
  - `[patch.crates-io]` local path is acceptable for local dev.

5) Minimal wording to include in PR / Codex prompts

- “Reject any WIT declaring `greentic:component@0.4.0` or `@0.5.0`. Migrate all remaining uses to `@0.6.0`.”
- “Reject any repo-local `.wit` redefining `world component-v0-v6-v0` under `greentic:component@0.6.0` outside `greentic-interfaces`.”
- “Providers and runner tests must consume canonical WIT; no local component WIT compilation in provider/runner tests.”

6) Migration note (repo-local implementation constraints)

- This repo currently has provider components exporting legacy-named interfaces (`descriptor`, `runtime`, `qa`) with operation-oriented runtime behavior.
- Current canonical 0.6.0 in `greentic-interfaces` uses `component-descriptor`, `component-schema`, `component-runtime`, `component-qa`, `component-i18n` with different runtime/QA signatures.
- To close policy violations without breaking behavior in one step:
  - Remove all repo-local canonical redefinitions by ensuring no local WIT in this repo declares `package greentic:component@0.6.0` together with `world component-v0-v6-v0` (including split files).
  - Move provider-local worlds to provider-owned package IDs (e.g. `provider:messaging-provider-slack@0.6.0`) until full ABI migration lands.
- Follow-up (separate migration pass):
  - Switch provider/runtime/test code to canonical interface names and signatures from `greentic-interfaces` 0.6.0.
  - Remove remaining local component-world compilation in tests.
