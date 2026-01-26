# Conventions & Build Pipeline Audit (greentic-messaging-providers)

This report is a repo-specific audit of current conventions and pipeline behavior. It is observational only.

## 1) Repo layout (authoritative paths)

### Component directories (authoritative)
Component directories live under `components/*` (glob: `components/*/component.manifest.json`).

Per-component paths (observed):

| component_dir | manifest | schemas | wit | flows | assets | notes |
| --- | --- | --- | --- | --- | --- | --- |
| `components/ai.greentic.component-templates` | `components/ai.greentic.component-templates/component.manifest.json` | `components/ai.greentic.component-templates/schemas/` | (none) | (none) | (none) | schema refs resolve under `schemas/io/*.schema.json` |
| `components/messaging-ingress-slack` | `components/messaging-ingress-slack/component.manifest.json` | (none) | `components/messaging-ingress-slack/wit/` | (none) | (none) | no `operations` block |
| `components/messaging-ingress-teams` | `components/messaging-ingress-teams/component.manifest.json` | (none) | `components/messaging-ingress-teams/wit/` | (none) | (none) | no `operations` block |
| `components/messaging-ingress-telegram` | `components/messaging-ingress-telegram/component.manifest.json` | (none) | `components/messaging-ingress-telegram/wit/` | (none) | (none) | no `operations` block |
| `components/messaging-ingress-whatsapp` | `components/messaging-ingress-whatsapp/component.manifest.json` | (none) | `components/messaging-ingress-whatsapp/wit/` | (none) | (none) | no `operations` block |
| `components/messaging-provider-*` | `components/messaging-provider-*/component.manifest.json` | `components/messaging-provider-*/schemas/` | `components/messaging-provider-*/wit/` | (none) | (none) | no `operations` block (provider manifests are metadata-only) |
| `components/provision` | `components/provision/component.manifest.json` | `components/provision/schemas/` | `components/provision/wit/` | (none) | (none) | `apply` op schema refs |
| `components/questions` | `components/questions/component.manifest.json` | `components/questions/schemas/` | (none) | (none) | (none) | `emit/validate/example-answers` op schema refs |
| `components/secrets-probe` | `components/secrets-probe/component.manifest.json` | (none) | `components/secrets-probe/wit/` | (none) | (none) | no `operations` block |
| `components/slack` | `components/slack/component.manifest.json` | (none) | `components/slack/wit/` | (none) | (none) | no `operations` block |
| `components/teams` | `components/teams/component.manifest.json` | (none) | `components/teams/wit/` | (none) | (none) | no `operations` block |
| `components/telegram` | `components/telegram/component.manifest.json` | (none) | `components/telegram/wit/` | (none) | (none) | no `operations` block |
| `components/templates` | `components/templates/component.manifest.json` | `components/templates/schemas/` | (none) | (none) | (none) | `text` op schema refs |
| `components/webchat` | `components/webchat/component.manifest.json` | (none) | `components/webchat/wit/` | (none) | (none) | no `operations` block |
| `components/webex` | `components/webex/component.manifest.json` | (none) | `components/webex/wit/` | (none) | (none) | no `operations` block |
| `components/whatsapp` | `components/whatsapp/component.manifest.json` | (none) | `components/whatsapp/wit/` | (none) | (none) | no `operations` block |

### Packs + flows
- Packs live under `packs/messaging-*/`.
- Flows live under `packs/messaging-*/flows/*.ygtc`.
- Flow resolve metadata under `packs/messaging-*/flows/*.ygtc.resolve*.json`.
- Pack manifests: `packs/messaging-*/pack.manifest.json` and `packs/messaging-*/pack.yaml`.
- Pack assets: `packs/messaging-*/assets/` and root `packs/messaging-*/setup.yaml`.

### Tree excerpt (top levels)
```
components/
  <component>/
    component.manifest.json
    schemas/            # if present
    wit/                # if present
packs/
  messaging-<provider>/
    flows/
      *.ygtc
      *.ygtc.resolve*.json
    components/         # packed wasm + manifests
    assets/
    pack.yaml
    pack.manifest.json
crates/
  greentic-messaging-packgen/
    src/main.rs
specs/
  providers/
    *.yaml
```

## 2) How flows are produced today

### Method A: packgen-driven flow generation (CLI)
- Entry: `crates/greentic-messaging-packgen/src/main.rs`.
- Commands used:
  - `greentic-flow new --flow <path> --id <id> --type job`
  - `greentic-flow add-step --flow <path> --node-id <id> --operation <op> --payload <json> --local-wasm <path>`
  - `greentic-flow update-step --flow <path> --step <id> --routing-next <id>`
  - `greentic-flow answers --component <manifest> --operation <op> --name <name> --out-dir <dir>`
  - `greentic-flow doctor <flow>` (validation)
- Inputs:
  - component manifests from `components/*/component.manifest.json`
  - wasm paths from `components/*/*.wasm`
  - packgen spec files `specs/providers/*.yaml`
- Outputs:
  - generated flows under `target/generated/providers/<pack_id>/flows/*.ygtc`
- Deterministic: Yes; generation is CLI-driven and uses a fixed spec + manifest paths.
- Wiring:
  - `crates/greentic-messaging-packgen/src/main.rs` (`generate_flows`, `flow_add_step`, `flow_update_routing`, `run_flow_doctor`).
  - CI test: `.github/workflows/build-and-publish.yml` “Packgen gtests” and `crates/greentic-messaging-packgen/tests/packgen.rs`.

### Method B: existing checked-in pack flows (source)
- `packs/messaging-*/flows/*.ygtc` are present and validated in CI, but are not regenerated by CI.
- Validation only:
  - `ci/local_check.sh` runs `greentic-flow doctor packs/*/flows/*.ygtc`.
  - workflows (`.github/workflows/*`) do the same.
- Deterministic: n/a (treated as source files).

**Inference:** flows under `packs/` are currently treated as source files (checked in) rather than generated artifacts, because CI validates them but does not regenerate/compare.

## 3) Where packgen is and what it does

### Entrypoints
- Binary: `crates/greentic-messaging-packgen` (CLI `greentic-messaging-packgen`).
- Spec catalog: `specs/providers/*.yaml`.
- CI tests: `.github/workflows/build-and-publish.yml` (Packgen gtests) and `crates/greentic-messaging-packgen/tests/packgen.rs`.

### Inputs/outputs
- Consumes:
  - Spec files (provider metadata + flows list) in `specs/providers/`.
  - Component manifests and wasm in `components/`.
  - Optional source pack directories (for assets/fixtures/schemas) defined in spec.
- Produces:
  - Generated pack dirs: `target/generated/providers/<pack_id>/`.
  - Pack artifacts when built: `dist/packs/<pack_id>.gtpack` (via `greentic-pack build`).
- Flow generation behavior:
  - packgen generates flows using `greentic-flow` CLI (not by copying pack flows).
  - packgen runs `greentic-flow doctor` after generation.
- Validation performed:
  - `greentic-flow doctor` during generation.
  - Pack build + doctor in tests (`crates/greentic-messaging-packgen/tests/packgen.rs`).

### Other pack generation paths
- `tools/publish_packs_oci.sh` builds packs using `greentic-pack` and then runs `greentic-pack doctor` (with messaging validator).
- `tools/build_packs.sh` is a wrapper around `tools/publish_packs_oci.sh` (dry-run).

## 4) Validation/doctor/strictness today

### `ci/local_check.sh`
- `cargo fmt --check`
- `tools/build_components.sh`
- `tools/check_op_schemas.py`
- `tools/sync_packs.sh`
- `greentic-flow doctor packs/*/flows/*.ygtc`
- `greentic-component doctor packs/*/components/*.manifest.json` (if present)
- `greentic-component test` for questions (emit/validate)
- `tools/publish_packs_oci.sh` (dry-run) + `greentic-pack doctor --validate` (validator pack) if `RUN_PUBLISH_PACKS` enabled
- `cargo test --workspace`

### GitHub Actions
- `.github/workflows/build-and-publish.yml`:
  - `python3 tools/check_op_schemas.py`
  - `greentic-flow doctor packs/*/flows/*.ygtc`
  - `greentic-component doctor packs/*/components/*.manifest.json`
  - `greentic-pack doctor --validate --validator-pack ...`
  - Packgen gtests: `cargo test -p greentic-messaging-packgen --tests`
- `.github/workflows/e2e-*.yml`:
  - `greentic-flow doctor` on pack flows
  - `greentic-component doctor` on pack components
  - `tools/publish_packs_oci.sh`

**Strictness toggles (observed):**
- `RUN_PUBLISH_PACKS` / `CI` controls whether publish dry-run is executed in `ci/local_check.sh`.
- No explicit strict/permissive flags observed in this repo for flow/pack validation.

## 5) Hand-rolled vs generated signals

**Checked-in flows (packs/):**
- Evidence of being treated as source:
  - `packs/*/flows/*.ygtc` are in repo.
  - CI validates them but does not regenerate them.
  - No `.gitignore` entries marking `.ygtc` as generated.
  - No script that overwrites `packs/*/flows` during CI.

**Generated flows (packgen output):**
- `target/generated/providers/*/flows/*.ygtc` are created by packgen and are not committed.
- Packgen uses `greentic-flow new/add-step/update-step/answers` (CLI) and runs doctor.

**Inference:**
- Source-of-truth flows are currently the checked-in flows under `packs/`.
- Packgen flows are generated artifacts for testing / packgen output only.

## 6) Current schema quality

Scan results from `components/**/component.manifest.json`:
- Components with `operations` now reference meaningful schemas:
  - `components/questions/component.manifest.json`: `emit`, `validate`, `example-answers` all reference structured schemas under `components/questions/schemas/`.
  - `components/provision/component.manifest.json`: `apply` references structured schemas under `components/provision/schemas/`.
  - `components/templates/component.manifest.json`: `text` references structured schemas under `components/templates/schemas/io/`.
  - `components/ai.greentic.component-templates/component.manifest.json`: `text` references structured schemas under `components/ai.greentic.component-templates/schemas/io/`.

- Components **without** `operations` blocks (schemas are n/a):
  - `components/messaging-ingress-*`, `components/messaging-provider-*`, `components/secrets-probe`, and `components/{slack,teams,telegram,webchat,webex,whatsapp}`.

Summary table (top offenders first):
- None currently failing the “meaningful schema” rule; all declared operations point to structured schemas.
- Components without `operations` are n/a (no op schemas to validate).

## 7) Proposed single canonical pipeline mapping (no implementation)

**Goal:** one canonical flow + pack generation path that is reproducible and validated.

Suggested pipeline (commands + locations):
1) **Generate flows via CLI**
   - Command: `greentic-messaging-packgen generate-all --spec-dir specs/providers --out target/generated/providers`
   - Location to add wrapper: `ci/gen_flows.sh`
2) **Validate generated flows**
   - Command: `greentic-flow doctor target/generated/providers/*/flows/*.ygtc`
   - Location: `ci/check_generated.sh`
3) **Build packs from generated output**
   - Command: `greentic-pack build --no-update --in target/generated/providers/<pack_id> --gtpack-out dist/packs/<pack_id>.gtpack`
4) **Doctor packs**
   - Command: `greentic-pack doctor --validate --validator-pack oci://ghcr.io/greentic-ai/validators/messaging:latest --pack dist/packs/*.gtpack`

CI hook points (specific):
- Add `ci/gen_flows.sh` before flow validation in `.github/workflows/build-and-publish.yml`.
- Add `ci/check_generated.sh` after generation to enforce reproducibility.
- Optionally remove validation against `packs/*/flows` once packgen is canonical.

Strict validation order (proposed):
1) `tools/check_op_schemas.py`
2) `greentic-flow doctor` (generated flows)
3) `greentic-component doctor` (manifest schema correctness)
4) `greentic-pack build` + `greentic-pack doctor --validate`

