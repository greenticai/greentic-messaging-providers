PR 1 — Fast-fail CI redesign for deterministic pack validation
Title

PR-CI-FASTFAIL-01: Add early deterministic pack validation and split pack checks from late build/publish

Goal

Stop discovering pack-lock / digest / staging problems only at the end of the pipeline.
Make CI fail in minutes when:

a pack lock references stale component bytes

a pack is being assembled from inconsistent sources

templates / flows / staged wasm drift from expected digests

Problem statement

Current CI validates packs too late:

components build in matrix

templates/flows get generated later

pack assembly happens late

validation errors such as PACK_LOCK_COMPONENT_DIGEST_MISMATCH only surface near the end

This causes:

20–30 minute feedback loops

hard-to-reproduce integration failures

hidden drift between local build artifacts and OCI-pulled content

Design

Introduce a new early pack validation stage between:

build-components

ensure-templates

gen-flows

and the later:

flow-doctor

component-doctor

questions-component-test

build-packs

publish

The new stage will:

download all built component wasm artifacts

stage pack inputs deterministically

regenerate or refresh pack lock inputs

run greentic-pack doctor on pack sources or generated pack artifacts

fail immediately on digest mismatches or stale lock data

Scope
Add new job

Add:

validate-pack-inputs

Add new script

Add:

ci/steps/07a_validate_pack_inputs.sh

Update dependencies

Make downstream jobs depend on validate-pack-inputs instead of only sync-packs.

Required workflow changes
File

.github/workflows/build-test-publish.yml

Changes
1. Add new job after gen-flows
  validate-pack-inputs:
    runs-on: ubuntu-latest
    needs: [build-components, ensure-templates, gen-flows]
    steps:
      - name: Checkout
        uses: actions/checkout@v4

      - name: Cache cargo bin tools
        uses: actions/cache@v4
        with:
          path: |
            ~/.cargo/bin
          key: cargo-bin-tools-${{ runner.os }}-v1

      - name: Install cargo-binstall
        run: |
          if ! command -v cargo-binstall >/dev/null 2>&1; then
            cargo install cargo-binstall --locked
          fi

      - name: Install greentic-pack
        env:
          CARGO_NET_GIT_FETCH_WITH_CLI: "true"
        run: |
          cargo binstall greentic-pack --force --no-confirm --locked || cargo install greentic-pack --force --locked
          echo "${HOME}/.cargo/bin" >> "${GITHUB_PATH}"
          greentic-pack --version

      - name: Install oras
        uses: oras-project/setup-oras@v1
        with:
          version: 1.2.0

      - name: Download component artifacts
        uses: actions/download-artifact@v4
        with:
          pattern: components-wasm-*
          path: target/components
          merge-multiple: true

      - name: Step 07a - Validate pack inputs early
        run: ./ci/steps/07a_validate_pack_inputs.sh

      - name: Upload early pack validation report on failure
        if: failure()
        uses: actions/upload-artifact@v4
        with:
          name: early-pack-validation-report
          path: |
            dist/packs/*.gtpack
            dist/pack_validation_report.json
            packs.lock.json
          if-no-files-found: ignore
2. Update sync-packs dependencies

Change:

needs: [build-components, ensure-templates, gen-flows]

to:

needs: [validate-pack-inputs]
3. Update later jobs to depend on the fast-fail stage

Where useful, change late jobs so the pack pipeline only proceeds if validation passed.

Examples:

flow-doctor

component-doctor

questions-component-test

build-packs

should continue to depend on the chain flowing through validate-pack-inputs.

New script
File

ci/steps/07a_validate_pack_inputs.sh

Responsibility

This script should:

ensure deterministic local staging of built wasm

ensure no pack build path depends on opportunistic remote mutation

run resolve/doctor before the expensive build stage

emit a useful report on failure

Proposed behavior
#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

mkdir -p dist
mkdir -p dist/packs

echo "== Stage locally built components into expected locations =="
./ci/steps/07_sync_packs.sh

echo "== Early validate pack inputs =="
REPORT="dist/pack_validation_report.json"
TMP_REPORT="$(mktemp)"
printf '[]' > "$TMP_REPORT"

failures=0

for pack_dir in packs/*; do
  [ -d "$pack_dir" ] || continue
  pack_name="$(basename "$pack_dir")"

  echo "--- validating ${pack_name} ---"

  tmp_out=".tmp/validate-${pack_name}.gtpack"
  mkdir -p .tmp

  if ! greentic-pack build \
      --pack "$pack_dir" \
      --output "$tmp_out"; then
    echo "pack build failed for ${pack_name}"
    failures=$((failures + 1))
    continue
  fi

  doctor_out="$(mktemp)"
  set +e
  greentic-pack doctor --pack "$tmp_out" --format json > "$doctor_out"
  rc=$?
  set -e

  cat "$doctor_out"

  if [ $rc -ne 0 ]; then
    echo "doctor failed for ${pack_name}"
    cp "$tmp_out" "dist/packs/${pack_name}.gtpack" || true
    failures=$((failures + 1))
  fi
done

if [ "$failures" -ne 0 ]; then
  echo "Early pack validation failed with ${failures} pack(s)"
  exit 1
fi

echo "Early pack validation passed"
Notes for Codex

Codex should adapt this to the actual greentic-pack CLI flags in the repo.
If greentic-pack build has a better dry-run/resolve/doctor sequence, use that instead.
The most important thing is that this script trips on digest mismatch before Step 11.

Acceptance criteria

A stale pack.lock.cbor or drifted component causes CI failure in validate-pack-inputs

Failures occur before late integration jobs

CI logs clearly show which pack failed

Failure artifacts contain enough material for local reproduction

PR 2 — Local workflow runners and developer fast paths
Title

PR-CI-LOCAL-02: Add local CI runner scripts for full, fast, and validation-only workflows

Goal

Allow local reproduction of the GitHub Actions pipeline without waiting for remote CI.

Requested scripts

Per your preference, add these four scripts:

./ci/run_all_actions.sh

./ci/run_fast_actions.sh

./ci/validate_packs.sh

./ci/rebuild_pack.sh

Design
run_all_actions.sh

Runs the full GitHub Actions workflow locally using act.

run_fast_actions.sh

Runs only the fast-fail CI path locally, focusing on the early pack validation section.

validate_packs.sh

Runs the deterministic shell path directly, without act, for fastest iteration.

rebuild_pack.sh

Rebuilds exactly one pack, optionally publishing it.

Files to add
1. ci/run_all_actions.sh
#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if ! command -v act >/dev/null 2>&1; then
  echo "act is not installed"
  exit 1
fi

SECRETS_FILE="${SECRETS_FILE:-.secrets.act}"
WORKFLOW="${WORKFLOW:-.github/workflows/build-test-publish.yml}"
EVENT="${EVENT:-pull_request}"

ARGS=(
  "$EVENT"
  --workflows "$WORKFLOW"
  --artifact-server-path .act-artifacts
  --pull=false
)

if [[ -f "$SECRETS_FILE" ]]; then
  ARGS+=(--secret-file "$SECRETS_FILE")
fi

exec act "${ARGS[@]}"
2. ci/run_fast_actions.sh

This should target just the quick path jobs. Since act supports -j, we can run the most relevant job.

#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if ! command -v act >/dev/null 2>&1; then
  echo "act is not installed"
  exit 1
fi

SECRETS_FILE="${SECRETS_FILE:-.secrets.act}"
WORKFLOW="${WORKFLOW:-.github/workflows/build-test-publish.yml}"
EVENT="${EVENT:-pull_request}"
JOB="${JOB:-validate-pack-inputs}"

ARGS=(
  "$EVENT"
  --workflows "$WORKFLOW"
  --artifact-server-path .act-artifacts
  --pull=false
  -j "$JOB"
)

if [[ -f "$SECRETS_FILE" ]]; then
  ARGS+=(--secret-file "$SECRETS_FILE")
fi

exec act "${ARGS[@]}"
3. ci/validate_packs.sh

This should be the fastest shell-only path.

#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

./ci/steps/04_ensure_templates.sh
./ci/steps/06_gen_flows.sh
./ci/steps/07a_validate_pack_inputs.sh
Optional enhancement

Support flags like:

./ci/validate_packs.sh --skip-flows
./ci/validate_packs.sh --pack messaging-dummy

That can come later if you want.

4. ci/rebuild_pack.sh

This is the manual fast lane.

CLI contract
./ci/rebuild_pack.sh <pack-name> [--publish]

Examples:

./ci/rebuild_pack.sh webchat-gui
./ci/rebuild_pack.sh webchat-gui --publish
./ci/rebuild_pack.sh messaging-dummy
Proposed implementation
#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if [ $# -lt 1 ]; then
  echo "usage: $0 <pack-name> [--publish]"
  exit 1
fi

PACK_NAME=""
PUBLISH=0

for arg in "$@"; do
  case "$arg" in
    --publish)
      PUBLISH=1
      ;;
    *)
      if [ -z "$PACK_NAME" ]; then
        PACK_NAME="$arg"
      else
        echo "unexpected argument: $arg"
        exit 1
      fi
      ;;
  esac
done

PACK_DIR="packs/${PACK_NAME}"
OUT_DIR="dist/packs"
OUT_PACK="${OUT_DIR}/${PACK_NAME}.gtpack"

if [ ! -d "$PACK_DIR" ]; then
  echo "pack not found: ${PACK_DIR}"
  exit 1
fi

mkdir -p "$OUT_DIR"

echo "== Ensure templates =="
./ci/steps/04_ensure_templates.sh

echo "== Generate flows =="
./ci/steps/06_gen_flows.sh

echo "== Sync/stage packs =="
./ci/steps/07_sync_packs.sh

echo "== Build pack ${PACK_NAME} =="
greentic-pack build --pack "$PACK_DIR" --output "$OUT_PACK"

echo "== Doctor pack ${PACK_NAME} =="
greentic-pack doctor --pack "$OUT_PACK"

echo "Built ${OUT_PACK}"

if [ "$PUBLISH" -eq 1 ]; then
  echo "== Publish pack ${PACK_NAME} =="
  ./ci/publish_pack.sh "$PACK_NAME"
fi
Acceptance criteria

developer can reproduce CI locally with one command

developer can run fast validation locally with one command

developer can rebuild a single pack in minutes

webchat-gui refresh becomes a fast path, not a full-pipeline event

PR 3 — Make pack assembly deterministic and stop late drift
Title

PR-CI-DETERMINISTIC-03: Remove mutable pack assembly inputs and force local/staged artifact use before publish

Goal

Prevent pack lock mismatches caused by drift between:

locally built component bytes

checked-in lockfiles

GHCR-pulled mutable tags such as :latest

Problem statement

Your logs show remote fetches during pack creation:

Fetching OCI component ghcr.io/greenticai/components/templates:latest...

That means the CI build path is not fully deterministic.

Even if the workflow succeeds most of the time, mutable remote pulls make pack resolution brittle.

Design principles

For all non-publish jobs:

prefer local built artifacts over remote OCI pulls

avoid :latest

do not re-fetch a template/component if a matching local artifact is already staged

only the publish job should push/pull versioned OCI artifacts intentionally

Required changes
1. Audit scripts that call remote component/template fetches

Likely candidates:

ci/steps/04_ensure_templates.sh

ci/steps/07_sync_packs.sh

ci/steps/11_build_packs.sh

any helper under tools/ used during pack generation

2. Add a common helper for deterministic local staging
File

ci/lib/stage_local_components.sh

Responsibility

verify target/components/*.wasm exist

copy/link them into pack/component staging locations expected by later steps

refuse to silently fall back to mutable remote fetches unless explicitly allowed

Example contract
#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

mkdir -p components/provision components/questions

copy_if_present() {
  local src="$1"
  local dst="$2"
  if [ -f "$src" ]; then
    mkdir -p "$(dirname "$dst")"
    cp "$src" "$dst"
    echo "staged $src -> $dst"
  fi
}

copy_if_present target/components/provision.wasm components/provision/provision.wasm
copy_if_present target/components/questions.wasm components/questions/questions.wasm

Codex should expand this for all pack-relevant built components if needed.

3. Gate remote pulls behind explicit environment variable

For example:

ALLOW_REMOTE_TEMPLATE_FETCH=1

Default in CI fast path should be off.

That way:

local validation uses local staged assets

full publish path may still intentionally resolve/publish

4. Replace :latest where feasible

If a remote pull is still required somewhere, replace:

ghcr.io/greenticai/components/templates:latest

with either:

workflow-produced digest from the just-published component artifact

preferred:

ghcr.io/greenticai/components/templates@sha256:...

acceptable fallback:

version tag from workspace version

digest-pinned reference

explicit CI-passed resolved version

Suggested env variable
env:
  COMPONENT_TEMPLATE_REF: ghcr.io/greenticai/components/templates@${{ needs.publish-components.outputs.templates_digest }}

If wiring job outputs is awkward, the publish workflow may instead read the
templates digest from `components.lock.json` produced earlier in the same run
and export the digest-pinned OCI ref through `GITHUB_ENV`.

Rationale

Using `PUBLISH_VERSION` is better than `:latest`, but still leaves room for
tag drift, republish ambiguity, and timing issues between the producer and
consumer steps. Using the digest emitted by the same workflow gives both:

- deterministic reruns
- automatic use of the newest artifact produced by that workflow
- no dependency on mutable tag state
- straightforward rollback/debuggability from the lockfile

Acceptance criteria

no fast-path CI job relies on mutable OCI tags

early validation and pack build use the same staged component bytes

digest mismatches caused by mixed local/remote sources disappear

publish remains the only place where remote registry state is intentionally updated

PR 4 — Optional pack build matrix for isolation and faster reruns
Title

PR-CI-PACK-MATRIX-04: Build packs in a matrix and isolate failures per pack

Goal

Avoid one bad pack blocking visibility into all pack build outcomes.

Design

Replace the single build-packs monolithic job with a matrix:

strategy:
  fail-fast: false
  matrix:
    pack:
      - messaging-dummy
      - messaging-slack
      - messaging-teams
      - messaging-telegram
      - messaging-webchat
      - messaging-webex
      - messaging-whatsapp
      - webchat-gui
Benefits

failure isolation

faster reruns

easier local reproduction

better artifact granularity

New script

ci/steps/11_build_one_pack.sh

Contract
./ci/steps/11_build_one_pack.sh messaging-dummy
Behavior

validate pack exists

ensure local staged artifacts are ready

build only that pack

doctor it

write dist/packs/<name>.gtpack

Acceptance criteria

one broken pack no longer hides results for all others

rerunning a failed pack is cheap

build artifacts are available per pack

PR 5 — Manual workflow_dispatch for rebuilding one pack
Title

PR-CI-MANUAL-05: Add workflow_dispatch for single-pack rebuild and optional publish

Goal

Enable quick GHCR updates for packs like webchat-gui.gtpack after greentic-webchat changes.

File

Add a new workflow, for example:

.github/workflows/rebuild-pack.yml

Example
name: Rebuild single pack

on:
  workflow_dispatch:
    inputs:
      pack:
        description: Pack name
        required: true
        type: string
      publish:
        description: Publish rebuilt pack
        required: true
        type: boolean
        default: false

permissions:
  contents: read
  packages: write

jobs:
  rebuild-pack:
    runs-on: ubuntu-latest
    steps:
      - name: Checkout
        uses: actions/checkout@v4

      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable
        with:
          targets: wasm32-wasip2

      - name: Cache Rust
        uses: Swatinem/rust-cache@v2
        with:
          key: rebuild-pack

      - name: Cache cargo bin tools
        uses: actions/cache@v4
        with:
          path: ~/.cargo/bin
          key: cargo-bin-tools-${{ runner.os }}-v1

      - name: Install cargo-binstall
        run: |
          if ! command -v cargo-binstall >/dev/null 2>&1; then
            cargo install cargo-binstall --locked
          fi

      - name: Install greentic-pack
        run: |
          cargo binstall greentic-pack --force --no-confirm --locked || cargo install greentic-pack --force --locked

      - name: Login to GHCR
        if: inputs.publish
        run: echo "${{ secrets.GITHUB_TOKEN }}" | oras login ghcr.io -u "${{ github.actor }}" --password-stdin

      - name: Rebuild selected pack
        run: ./ci/rebuild_pack.sh "${{ inputs.pack }}"

      - name: Publish selected pack
        if: inputs.publish
        run: ./ci/rebuild_pack.sh "${{ inputs.pack }}" --publish
Acceptance criteria

maintainers can rebuild webchat-gui on demand

no need to rerun the full main pipeline just to refresh one pack

publish remains explicit and auditable

Recommended merge order

I would merge these in this order:

First

PR-CI-FASTFAIL-01
This gives immediate value.

Second

PR-CI-LOCAL-02
This gives you quick local feedback loops.

Third

PR-CI-DETERMINISTIC-03
This removes the root cause of drift.

Fourth

PR-CI-MANUAL-05
This gives the webchat fast lane.

Fifth

PR-CI-PACK-MATRIX-04
Useful, but can come after the earlier fixes.

What I would tell Codex to pay special attention to
1. Do not invent CLI flags

Codex should inspect actual usage of:

greentic-pack build

greentic-pack doctor

greentic-pack resolve

and adapt the scripts to real flags.

2. Preserve current repo conventions

If the repo already has helper libraries under ci/lib or tools/, reuse them instead of introducing conflicting patterns.

3. Make deterministic local staging the default

Fast-path jobs should not silently fetch mutable remote inputs.

4. Keep publish behavior separate

Do not mix “validate locally” and “publish remotely” logic.

Very short Codex brief

You can give Codex this compact instruction:

Implement five PRs in greentic-messaging-providers:

Add early validate-pack-inputs CI stage using ci/steps/07a_validate_pack_inputs.sh so pack lock digest mismatches fail before late jobs.

Add local scripts: ci/run_all_actions.sh, ci/run_fast_actions.sh, ci/validate_packs.sh, and ci/rebuild_pack.sh.

Refactor pack assembly to prefer local staged artifacts and avoid mutable remote pulls in fast-path CI jobs.

Optionally split build-packs into a pack matrix using ci/steps/11_build_one_pack.sh.

Add workflow_dispatch single-pack rebuild/publish workflow for fast webchat-gui.gtpack refreshes.
Reuse existing repo patterns, inspect actual greentic-pack CLI flags before scripting, and keep publish logic separate from validation.
