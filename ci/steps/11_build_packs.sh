#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT_DIR}"

GREENTIC_PACK_MIN_VERSION="0.4.89"
GREENTIC_PACK_VERSION="${GREENTIC_PACK_VERSION:-^0.4}"

# Ensure greentic-pack >= GREENTIC_PACK_MIN_VERSION (add-extension requires 0.4.89+)
needs_upgrade=0
if command -v greentic-pack >/dev/null 2>&1; then
  current="$(greentic-pack --version | awk '{print $NF}')"
  cur_patch="${current##0.4.}"
  min_patch="${GREENTIC_PACK_MIN_VERSION##0.4.}"
  if [ "${cur_patch}" -lt "${min_patch}" ] 2>/dev/null; then
    echo "greentic-pack ${current} is too old (need >= ${GREENTIC_PACK_MIN_VERSION}), removing stale binary..."
    rm -f "$(command -v greentic-pack)"
    needs_upgrade=1
  else
    echo "Using existing greentic-pack: greentic-pack ${current}"
  fi
else
  needs_upgrade=1
fi

if [ "${needs_upgrade}" -eq 1 ]; then
  if ! command -v cargo-binstall >/dev/null 2>&1; then
    cargo install cargo-binstall --locked
  fi
  cargo binstall greentic-pack --version "${GREENTIC_PACK_VERSION}" --force --no-confirm --locked || \
    cargo install greentic-pack --version "${GREENTIC_PACK_VERSION}" --force --locked
fi
PACK_VERSION="${PACK_VERSION:-$(python3 - <<'PY'
from pathlib import Path
import tomllib
data = tomllib.loads(Path("Cargo.toml").read_text())
print(data.get("workspace", {}).get("package", {}).get("version", "0.0.0"))
PY
)}"
export PACK_VERSION

validator_ref="oci://ghcr.io/greenticai/validators/messaging:latest"
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
  local raw_json
  local filtered_json
  local raw_stderr
  local doctor_rc=0
  local filtered_content
  local has_errors
  local raw_errors
  local filtered_errors
  raw_json="$(mktemp)"
  filtered_json="$(mktemp)"
  raw_stderr="$(mktemp)"

  if [ -n "${validator_file}" ] && pack_doctor_supports_validator_wasm && pack_doctor_supports_validator_policy; then
    if pack_doctor_supports_validate; then
      if ! greentic-pack doctor --json --validate --validator-wasm "greentic.validators.messaging=${validator_file}" --validator-policy required --pack "${pack_path}" >"${raw_json}" 2>"${raw_stderr}"; then
        doctor_rc=$?
      fi
    else
      if ! greentic-pack doctor --json --validator-wasm "greentic.validators.messaging=${validator_file}" --validator-policy required --pack "${pack_path}" >"${raw_json}" 2>"${raw_stderr}"; then
        doctor_rc=$?
      fi
    fi
  else
    if [ -n "${validator_file}" ] && [ "${validator_flag_warning_printed}" -eq 0 ]; then
      echo "warning: greentic-pack doctor lacks validator flags; running without external validator" >&2
      validator_flag_warning_printed=1
    fi

    if pack_doctor_supports_validate; then
      if ! greentic-pack doctor --json --validate --pack "${pack_path}" >"${raw_json}" 2>"${raw_stderr}"; then
        doctor_rc=$?
      fi
    else
      if ! greentic-pack doctor --json --pack "${pack_path}" >"${raw_json}" 2>"${raw_stderr}"; then
        doctor_rc=$?
      fi
    fi
  fi

  python3 "${ROOT_DIR}/tools/filter_pack_doctor_json.py" "${raw_json}" >"${filtered_json}"
  raw_errors="$(jq '(.validation.diagnostics // []) | map(select(.severity == "error")) | length' "${raw_json}")"
  filtered_errors="$(jq '(.validation.diagnostics // []) | map(select(.severity == "error")) | length' "${filtered_json}")"
  if [ "${raw_errors}" -gt "${filtered_errors}" ]; then
    echo "warning: ignored legacy helper doctor diagnostics for ${pack_path}" >&2
  fi
  filtered_content="$(cat "${filtered_json}")"
  has_errors="$(jq -r '.validation.has_errors // false' <<<"${filtered_content}")"
  if [ "${has_errors}" = "true" ]; then
    cat "${raw_stderr}" >&2
    rm -f "${raw_json}" "${filtered_json}" "${raw_stderr}"
    if [ "${doctor_rc}" -ne 0 ]; then
      return "${doctor_rc}"
    fi
    return 1
  fi
  rm -f "${raw_json}" "${filtered_json}" "${raw_stderr}"
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
