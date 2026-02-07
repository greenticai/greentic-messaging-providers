#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

PACK_VERSION="${PACK_VERSION:-$(python3 - <<'PY'
from pathlib import Path
import tomllib
data = tomllib.loads(Path("Cargo.toml").read_text())
print(data.get("workspace", {}).get("package", {}).get("version", "0.0.0"))
PY
)}"
export PACK_VERSION
export GREENTIC_RUNNER_SMOKE=1

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
./ci/steps/11_build_packs.sh

echo "==> cargo test --workspace"
# If this fails, re-run only:
#   ./ci/steps/12_cargo_test.sh
./ci/steps/12_cargo_test.sh

echo "All checks completed."
