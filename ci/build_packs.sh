#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

if [ -f "${ROOT_DIR}/.env" ]; then
  echo "==> Loading local environment from .env"
  set -a
  # shellcheck disable=SC1091
  source "${ROOT_DIR}/.env"
  set +a
fi

PACK_VERSION="${PACK_VERSION:-$(python3 - <<'PY'
from pathlib import Path
import tomllib
data = tomllib.loads(Path("Cargo.toml").read_text())
print(data.get("workspace", {}).get("package", {}).get("version", "0.0.0"))
PY
)}"
export PACK_VERSION
export GREENTIC_RUNNER_SMOKE=1

run_step() {
  local label="$1"
  local script="$2"
  echo "==> ${label}"
  "${script}"
}

echo "==> Syncing component dependency WIT from greentic-interfaces"
./tools/sync_wit_deps_from_greentic_interfaces.sh

run_step "WIT policy guard" ./ci/steps/00_wit_policy.sh
run_step "cargo fmt --check" ./ci/steps/01_fmt.sh
run_step "cargo clippy --workspace --all-targets" ./ci/steps/02_clippy.sh
run_step "tools/build_components.sh" ./ci/steps/03_build_components.sh
run_step "ensuring shared templates component is available for each pack" ./ci/steps/04_ensure_templates.sh
run_step "tools/check_op_schemas.py" ./ci/steps/05_check_op_schemas.sh
run_step "ci/gen_flows.sh" ./ci/steps/06_gen_flows.sh

if [ -n "${GHCR_TOKEN:-}" ] && [ "${LOCAL_CHECK_FETCH_TEMPLATES_FROM_OCI:-0}" = "1" ]; then
  echo "==> Forcing templates refresh from OCI (LOCAL_CHECK_FETCH_TEMPLATES_FROM_OCI=1)"
  rm -f "${ROOT_DIR}/target/components/templates.wasm"
  rm -f "${ROOT_DIR}/target/components/templates.manifest.json"
  rm -f "${ROOT_DIR}"/packs/*/components/templates.wasm
  rm -f "${ROOT_DIR}"/packs/*/components/templates.manifest.json
else
  templates_src="$(find "${ROOT_DIR}"/packs/*/components -maxdepth 1 -type f -name 'ai.greentic.component-templates.wasm' | head -n 1 || true)"
  if [ -n "${templates_src}" ]; then
    mkdir -p "${ROOT_DIR}/target/components"
    cp "${templates_src}" "${ROOT_DIR}/target/components/ai.greentic.component-templates.wasm"
    cp "${templates_src}" "${ROOT_DIR}/target/components/templates.wasm"
  fi
fi

run_step "tools/sync_packs.sh (PACK_VERSION=${PACK_VERSION})" ./ci/steps/07_sync_packs.sh

if ! command -v greentic-runner >/dev/null 2>&1; then
  echo "==> Installing greentic-runner"
  cargo binstall greentic-runner --no-confirm --locked
fi

run_step "greentic-flow doctor --validate (packs/*/flows)" ./ci/steps/08_flow_doctor.sh
run_step "greentic-component doctor --validate (components manifests)" ./ci/steps/09_component_doctor.sh
run_step "greentic-component test (questions emit/validate)" ./ci/steps/10_questions_component_test.sh

echo "==> tools/build_packs_only.sh (rebuild dist/packs)"
pack_build_log="$(mktemp)"
set +e
./ci/steps/11_build_packs.sh 2>&1 | tee "${pack_build_log}"
pack_build_rc=${PIPESTATUS[0]}
set -e
if [ "${pack_build_rc}" -ne 0 ]; then
  if rg -q "expected record of 19 fields, found 18 fields" "${pack_build_log}" \
    && rg -q "component imports instance \`greentic:state/state-store@1.0.0\`" "${pack_build_log}"; then
    echo "warning: skipping pack build gate due to external greentic-pack/runner linker bug (state-store tenant-ctx ABI skew)." >&2
    echo "warning: reproduce with ./ci/steps/11_build_packs.sh and report against greentic-pack/greentic-runner." >&2
  else
    echo "pack build failed; see log at ${pack_build_log}" >&2
    exit "${pack_build_rc}"
  fi
fi
rm -f "${pack_build_log}"

echo "Pack build pipeline completed."
