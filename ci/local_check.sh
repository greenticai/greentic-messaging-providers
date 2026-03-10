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

# Keep authentication identity (GHCR_USERNAME) decoupled from namespace
# selection. Namespace resolution is handled in tools/sync_packs.sh via
# TEMPLATES_NAMESPACE/GHCR_NAMESPACE/OCI_ORG.

PACK_VERSION="${PACK_VERSION:-$(python3 - <<'PY'
from pathlib import Path
import tomllib
data = tomllib.loads(Path("Cargo.toml").read_text())
print(data.get("workspace", {}).get("package", {}).get("version", "0.0.0"))
PY
)}"
export PACK_VERSION
export GREENTIC_RUNNER_SMOKE=1

echo "==> Syncing component dependency WIT from greentic-interfaces"
./tools/sync_wit_deps_from_greentic_interfaces.sh

echo "==> WIT policy guard"
# If this fails, re-run only:
#   ./ci/steps/00_wit_policy.sh
./ci/steps/00_wit_policy.sh

echo "==> cargo fmt --check"
# If this fails, re-run only:
#   ./ci/steps/01_fmt.sh
./ci/steps/01_fmt.sh

echo "==> cargo clippy --workspace --all-targets"
# If this fails, re-run only:
#   ./ci/steps/02_clippy.sh
./ci/steps/02_clippy.sh

echo "==> tools/build_components.sh"
# If this fails, re-run only:
#   ./ci/steps/03_build_components.sh
./ci/steps/03_build_components.sh

echo "==> ensuring shared templates component is available for each pack"
# If this fails, re-run only:
#   ./ci/steps/04_ensure_templates.sh
./ci/steps/04_ensure_templates.sh

echo "==> tools/check_op_schemas.py"
# If this fails, re-run only:
#   ./ci/steps/05_check_op_schemas.sh
./ci/steps/05_check_op_schemas.sh

echo "==> ci/gen_flows.sh"
# If this fails, re-run only:
#   ./ci/steps/06_gen_flows.sh
./ci/steps/06_gen_flows.sh

if [ -n "${GHCR_TOKEN:-}" ] && [ "${LOCAL_CHECK_FETCH_TEMPLATES_FROM_OCI:-0}" = "1" ]; then
  echo "==> Forcing templates refresh from OCI (LOCAL_CHECK_FETCH_TEMPLATES_FROM_OCI=1)"
  rm -f "${ROOT_DIR}/target/components/templates.wasm"
  rm -f "${ROOT_DIR}/target/components/templates.manifest.json"
  rm -f "${ROOT_DIR}"/packs/*/components/templates.wasm
  rm -f "${ROOT_DIR}"/packs/*/components/templates.manifest.json
else
  # Local/offline path: seed template artifacts from already-synced pack copies
  # so sync_packs won't require OCI fetch for template components.
  templates_src="$(find "${ROOT_DIR}"/packs/*/components -maxdepth 1 -type f -name 'ai.greentic.component-templates.wasm' | head -n 1 || true)"
  if [ -n "${templates_src}" ]; then
    mkdir -p "${ROOT_DIR}/target/components"
    cp "${templates_src}" "${ROOT_DIR}/target/components/ai.greentic.component-templates.wasm"
    cp "${templates_src}" "${ROOT_DIR}/target/components/templates.wasm"
  fi
fi

echo "==> tools/sync_packs.sh (PACK_VERSION=${PACK_VERSION})"
# If this fails, re-run only:
#   ./ci/steps/07_sync_packs.sh
./ci/steps/07_sync_packs.sh

if ! command -v greentic-runner >/dev/null 2>&1; then
  echo "==> Installing greentic-runner"
  cargo binstall greentic-runner --no-confirm --locked
fi

echo "==> greentic-flow doctor --validate (packs/*/flows)"
# If this fails, re-run only:
#   ./ci/steps/08_flow_doctor.sh
./ci/steps/08_flow_doctor.sh

echo "==> greentic-component doctor --validate (components manifests)"
# If this fails, re-run only:
#   ./ci/steps/09_component_doctor.sh
./ci/steps/09_component_doctor.sh

echo "==> greentic-component test (questions emit/validate)"
# If this fails, re-run only:
#   ./ci/steps/10_questions_component_test.sh
./ci/steps/10_questions_component_test.sh

echo "==> tools/build_packs_only.sh (dry-run; rebuild dist/packs)"
# If this fails, re-run only:
#   ./ci/steps/11_build_packs.sh
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

GREENTIC_PACK_TEST_VERSION="${GREENTIC_PACK_TEST_VERSION:-0.4}"
if ! command -v cargo-binstall >/dev/null 2>&1; then
  cargo install cargo-binstall --locked
fi
cargo binstall greentic-pack --version "${GREENTIC_PACK_TEST_VERSION}" --force --no-confirm --locked || \
  cargo install greentic-pack --version "${GREENTIC_PACK_TEST_VERSION}" --force --locked

echo "==> cargo test --workspace"
# If this fails, re-run only:
#   ./ci/steps/12_cargo_test.sh
cargo_test_log="$(mktemp)"
set +e
./ci/steps/12_cargo_test.sh 2>&1 | tee "${cargo_test_log}"
cargo_test_rc=${PIPESTATUS[0]}
set -e
if [ "${cargo_test_rc}" -ne 0 ]; then
  echo "cargo test failed; see log at ${cargo_test_log}" >&2
  exit "${cargo_test_rc}"
fi
rm -f "${cargo_test_log}"

echo "All checks completed."
