#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT_DIR}"

PACK_VERSION="${PACK_VERSION:-$(python3 - <<'PY'
from pathlib import Path
import tomllib
data = tomllib.loads(Path("Cargo.toml").read_text())
print(data.get("workspace", {}).get("package", {}).get("version", "0.0.0"))
PY
)}"
export PACK_VERSION

validator_ref="oci://ghcr.io/greentic-ai/validators/messaging:latest"
validator_root="${ROOT_DIR}/.greentic/validators"
validator_wasm="${validator_root}/greentic.validators.messaging.wasm"
mkdir -p "${validator_root}"
if command -v greentic-dev >/dev/null 2>&1; then
  if greentic-dev store fetch "${validator_ref}" --out "${validator_wasm}" >/dev/null 2>&1; then
    echo "Validator cached at ${validator_wasm}"
  else
    echo "Validator fetch skipped; using cached copy if present"
  fi
fi

run_publish_packs="${RUN_PUBLISH_PACKS:-${CI:-0}}"
case "${run_publish_packs}" in
  1|true|TRUE|yes|YES) run_publish_packs=1 ;;
  *) run_publish_packs=0 ;;
esac

if [ "${run_publish_packs}" -eq 1 ]; then
  if ! command -v cargo-binstall >/dev/null 2>&1; then
    cargo install cargo-binstall --locked
  fi
  DRY_RUN=1 PACK_VERSION="${PACK_VERSION}" PACKC_BUILD_FLAGS="${PACKC_BUILD_FLAGS:-}" ./tools/build_packs_only.sh
  if compgen -G "dist/packs/messaging-*.gtpack" >/dev/null; then
    for p in dist/packs/messaging-*.gtpack; do
      if [ -f "${validator_wasm}" ]; then
        greentic-pack doctor --validate --validator-wasm "greentic.validators.messaging=${validator_wasm}" --validator-policy required --pack "$p"
      else
        greentic-pack doctor --validate --pack "$p"
      fi
    done
  fi
else
  PACKC_BUILD_FLAGS="${PACKC_BUILD_FLAGS:-}"
  PACKC_BUILD_FLAGS="${PACKC_BUILD_FLAGS}" ./tools/build_packs_only.sh
fi
