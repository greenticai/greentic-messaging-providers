#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="${ROOT_DIR}/target/components"
BUILD_TARGET="wasm32-wasip2"
TARGET_DIR_OVERRIDE="${ROOT_DIR}/target/${BUILD_TARGET}"
PACKAGES=("provision" "questions" "secrets-probe" "slack" "teams" "telegram" "webchat" "webex" "webex-webhook" "whatsapp" "messaging-ingress-slack" "messaging-ingress-teams" "messaging-ingress-telegram" "messaging-ingress-whatsapp" "messaging-provider-dummy" "messaging-provider-telegram" "messaging-provider-teams" "messaging-provider-email" "messaging-provider-slack" "messaging-provider-webex" "messaging-provider-whatsapp" "messaging-provider-webchat")
BUILD_JOBS="${BUILD_COMPONENTS_JOBS:-1}"
WASM_TOOLS_BIN="${WASM_TOOLS_BIN:-wasm-tools}"
# Skip WASI preview 2 validation checks when requested.
SKIP_WASM_TOOLS_VALIDATION="${SKIP_WASM_TOOLS_VALIDATION:-0}"
HAS_WASM_TOOLS=0
# Keep tool caches inside the workspace to avoid sandbox write issues.
export XDG_CACHE_HOME="${XDG_CACHE_HOME:-${ROOT_DIR}/.cache}"
mkdir -p "${XDG_CACHE_HOME}"

if ! rustup target list --installed | grep -q "${BUILD_TARGET}"; then
  echo "Installing Rust target ${BUILD_TARGET}..."
  rustup target add "${BUILD_TARGET}"
fi

if ! command -v cargo-component >/dev/null 2>&1; then
  echo "cargo-component not found; installing..."
  cargo install cargo-component --locked
fi

if command -v "${WASM_TOOLS_BIN}" >/dev/null 2>&1; then
  HAS_WASM_TOOLS=1
else
  echo "wasm-tools not found; installing via cargo-binstall (fallback to cargo install if needed)..."
  if command -v cargo-binstall >/dev/null 2>&1; then
    cargo binstall --no-confirm --locked wasm-tools || true
  fi
  if command -v "${WASM_TOOLS_BIN}" >/dev/null 2>&1; then
    HAS_WASM_TOOLS=1
  else
    echo "cargo-binstall not available or wasm-tools still missing; attempting cargo install..."
    cargo install wasm-tools --locked || true
    if command -v "${WASM_TOOLS_BIN}" >/dev/null 2>&1; then
      HAS_WASM_TOOLS=1
    else
      echo "wasm-tools still not found; skipping WASI preview 2 validation checks (install wasm-tools to enable)" >&2
    fi
  fi
fi

mkdir -p "${TARGET_DIR}"
mkdir -p "${TARGET_DIR_OVERRIDE}"
mkdir -p "${TARGET_DIR_OVERRIDE}/wasm32-wasip1/release/deps"
mkdir -p "${TARGET_DIR_OVERRIDE}/wasm32-wasip1/debug/deps"
mkdir -p "${TARGET_DIR_OVERRIDE}/wasm32-wasip2/release/deps"
mkdir -p "${TARGET_DIR_OVERRIDE}/wasm32-wasip2/debug/deps"

build_one() {
  local package_name="$1"
  local artifact_name="${package_name//-/_}.wasm"
  local package_target_dir="${TARGET_DIR_OVERRIDE}/${package_name}"
  local artifact_path="${package_target_dir}/release/${artifact_name}"
  local nested_artifact_path="${package_target_dir}/${BUILD_TARGET}/release/${artifact_name}"

  cargo component build --release --package "${package_name}" --target "${BUILD_TARGET}" --target-dir "${package_target_dir}"

  if [ ! -f "${artifact_path}" ] && [ -f "${nested_artifact_path}" ]; then
    artifact_path="${nested_artifact_path}"
  fi

  if [ ! -f "${artifact_path}" ]; then
    echo "Expected artifact not found: ${artifact_path}" >&2
    return 1
  fi

  cp "${artifact_path}" "${TARGET_DIR}/${package_name}.wasm"
  if [ "${package_name}" = "provision" ]; then
    cp "${artifact_path}" "${ROOT_DIR}/components/provision/provision.wasm"
  fi
  if [ "${package_name}" = "questions" ]; then
    cp "${artifact_path}" "${ROOT_DIR}/components/questions/questions.wasm"
  fi
  if [ "${HAS_WASM_TOOLS}" -eq 1 ] && [ "${SKIP_WASM_TOOLS_VALIDATION}" -eq 0 ]; then
    if ! "${WASM_TOOLS_BIN}" component wit "${TARGET_DIR}/${package_name}.wasm" | grep -q "wasi:cli/"; then
      echo "Artifact ${package_name} does not appear to target WASI preview 2 (missing wasi:cli import)" >&2
      return 1
    fi
    "${WASM_TOOLS_BIN}" validate "${TARGET_DIR}/${package_name}.wasm" >/dev/null
  fi
  echo "Built ${TARGET_DIR}/${package_name}.wasm"
}

export ROOT_DIR TARGET_DIR TARGET_DIR_OVERRIDE BUILD_TARGET WASM_TOOLS_BIN HAS_WASM_TOOLS SKIP_WASM_TOOLS_VALIDATION
export -f build_one

if [ "${BUILD_JOBS}" -le 1 ]; then
  for package in "${PACKAGES[@]}"; do
    build_one "${package}"
  done
else
  if xargs -P 1 -n 1 echo >/dev/null 2>&1; then
    printf '%s\n' "${PACKAGES[@]}" | xargs -n 1 -P "${BUILD_JOBS}" bash -c 'build_one "$@"' _
  else
    echo "xargs -P not available; running builds sequentially" >&2
    for package in "${PACKAGES[@]}"; do
      build_one "${package}"
    done
  fi
fi

# Note: do not delete nested target triples here. This script is invoked from
# multiple test binaries in parallel, and deleting shared target directories can
# race with active builds (leading to missing .fingerprint files).
