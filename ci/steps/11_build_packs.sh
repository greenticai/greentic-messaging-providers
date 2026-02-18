#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT_DIR}"

if ! command -v cargo-binstall >/dev/null 2>&1; then
  cargo install cargo-binstall --locked
fi

GREENTIC_PACK_VERSION="${GREENTIC_PACK_VERSION:-0.4}"
cargo binstall greentic-pack --version "${GREENTIC_PACK_VERSION}" --force --no-confirm --locked || \
  cargo install greentic-pack --version "${GREENTIC_PACK_VERSION}" --force --locked

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

pack_doctor_supports_validate() {
  greentic-pack doctor --help 2>&1 | rg -q -- '--validate'
}

pack_doctor_supports_validator_wasm() {
  greentic-pack doctor --help 2>&1 | rg -q -- '--validator-wasm'
}

pack_doctor_supports_validator_policy() {
  greentic-pack doctor --help 2>&1 | rg -q -- '--validator-policy'
}

validator_flag_warning_printed=0
run_pack_doctor() {
  local pack_path="$1"
  local validator_file="${2:-}"

  if [ -n "${validator_file}" ] && pack_doctor_supports_validator_wasm && pack_doctor_supports_validator_policy; then
    if pack_doctor_supports_validate; then
      greentic-pack doctor --validate --validator-wasm "greentic.validators.messaging=${validator_file}" --validator-policy required --pack "${pack_path}"
    else
      greentic-pack doctor --validator-wasm "greentic.validators.messaging=${validator_file}" --validator-policy required --pack "${pack_path}"
    fi
    return
  fi

  if [ -n "${validator_file}" ] && [ "${validator_flag_warning_printed}" -eq 0 ]; then
    echo "warning: greentic-pack doctor lacks validator flags; running without external validator" >&2
    validator_flag_warning_printed=1
  fi

  if pack_doctor_supports_validate; then
    greentic-pack doctor --validate --pack "${pack_path}"
  else
    greentic-pack doctor --pack "${pack_path}"
  fi
}

if [ "${run_publish_packs}" -eq 1 ]; then
  if ! command -v cargo-binstall >/dev/null 2>&1; then
    cargo install cargo-binstall --locked
  fi
  DRY_RUN=1 PACK_VERSION="${PACK_VERSION}" PACKC_BUILD_FLAGS="${PACKC_BUILD_FLAGS:-}" ./tools/build_packs_only.sh
  if compgen -G "dist/packs/messaging-*.gtpack" >/dev/null; then
    for p in dist/packs/messaging-*.gtpack; do
      if [ -f "${validator_wasm}" ]; then
        run_pack_doctor "$p" "${validator_wasm}"
      else
        run_pack_doctor "$p"
      fi
    done
  fi
else
  PACKC_BUILD_FLAGS="${PACKC_BUILD_FLAGS:-}"
  PACKC_BUILD_FLAGS="${PACKC_BUILD_FLAGS}" ./tools/build_packs_only.sh
fi
