#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT_DIR}"

GREENTIC_PACK_VERSION="${GREENTIC_PACK_VERSION:-0.4.111}"
installed_pack_version=""
if command -v greentic-pack >/dev/null 2>&1; then
  installed_pack_version="$(greentic-pack --version | awk '{print $2}')"
  echo "Using existing greentic-pack: $(greentic-pack --version)"
fi
if [ -z "${installed_pack_version}" ] || [ "${installed_pack_version}" != "${GREENTIC_PACK_VERSION}" ]; then
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
PACK_FILTER="${PACK_FILTER:-}"

pack_selected() {
  local pack_name="$1"
  if [ -z "${PACK_FILTER}" ]; then
    return 0
  fi
  python3 - <<'PY' "${PACK_FILTER}" "${pack_name}"
import sys

raw = sys.argv[1]
name = sys.argv[2]
items = [part.strip() for chunk in raw.split(",") for part in chunk.split() if part.strip()]
raise SystemExit(0 if name in items else 1)
PY
}

refresh_flow_resolve_sidecars() {
  if ! command -v greentic-flow >/dev/null 2>&1; then
    return 0
  fi

  local pack_dir flow_file pack_name
  for pack_dir in packs/*; do
    [ -d "${pack_dir}" ] || continue
    pack_name="$(basename "${pack_dir}")"
    if ! pack_selected "${pack_name}"; then
      continue
    fi
    [ -d "${pack_dir}/flows" ] || continue
    find "${pack_dir}/flows" -maxdepth 1 -type f -name '*.resolve.summary.json' -delete
    for flow_file in "${pack_dir}"/flows/*.ygtc; do
      [ -f "${flow_file}" ] || continue
      greentic-flow doctor "${flow_file}" >/dev/null
    done
  done
}

pack_doctor_supports_validate() {
  local output
  output="$(greentic-pack doctor --validate --pack /nonexistent 2>&1 || true)"
  ! printf '%s' "${output}" | grep -Fq "unexpected argument '--validate'"
}

pack_doctor_supports_validator_wasm() {
  local output
  output="$(greentic-pack doctor --validator-wasm greentic.validators.messaging=/nonexistent --pack /nonexistent 2>&1 || true)"
  ! printf '%s' "${output}" | grep -Fq "unexpected argument '--validator-wasm'"
}

pack_doctor_supports_validator_policy() {
  local output
  output="$(greentic-pack doctor --validator-wasm greentic.validators.messaging=/nonexistent --validator-policy required --pack /nonexistent 2>&1 || true)"
  ! printf '%s' "${output}" | grep -Fq "unexpected argument '--validator-policy'"
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
    echo "Filtered pack doctor diagnostics for ${pack_path}:" >&2
    jq '.validation.diagnostics // []' <<<"${filtered_content}" >&2 || printf '%s\n' "${filtered_content}" >&2
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
      pack_name="$(basename "${p}" .gtpack)"
      if ! pack_selected "${pack_name}"; then
        continue
      fi
      if [ -f "${validator_wasm}" ]; then
        run_pack_doctor "$p" "${validator_wasm}"
      else
        run_pack_doctor "$p"
      fi
    done
  fi
else
  PACKC_BUILD_FLAGS="${PACKC_BUILD_FLAGS:-}"
  refresh_flow_resolve_sidecars
  PACKC_BUILD_FLAGS="${PACKC_BUILD_FLAGS}" ./tools/build_packs_only.sh
fi
