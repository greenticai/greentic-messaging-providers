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

run_step() {
  local label="$1"
  local script="$2"
  echo "==> ${label}"
  "${script}"
}

echo "==> Syncing component dependency WIT from greentic-interfaces"
./tools/sync_wit_deps_from_greentic_interfaces.sh

run_step "tools/build_components.sh" ./ci/steps/03_build_components.sh
run_step "ensuring shared templates component is available for each pack" ./ci/steps/04_ensure_templates.sh
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
run_step "tools/build_packs_only.sh" ./ci/steps/11_build_packs.sh

echo "Pack artifacts are available under dist/packs/"
