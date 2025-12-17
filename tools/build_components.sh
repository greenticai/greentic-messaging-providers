#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="${ROOT_DIR}/target/components"
BUILD_TARGET="wasm32-wasip2"
TARGET_DIR_OVERRIDE="${ROOT_DIR}/target/${BUILD_TARGET}"
PACKAGES=("secrets-probe" "slack" "teams" "telegram" "webchat" "webex" "whatsapp")

if ! rustup target list --installed | grep -q "${BUILD_TARGET}"; then
  echo "Installing Rust target ${BUILD_TARGET}..."
  rustup target add "${BUILD_TARGET}"
fi

mkdir -p "${TARGET_DIR}"
mkdir -p "${TARGET_DIR_OVERRIDE}"

for PACKAGE_NAME in "${PACKAGES[@]}"; do
  ARTIFACT_NAME="${PACKAGE_NAME//-/_}.wasm"
  ARTIFACT_PATH="${TARGET_DIR_OVERRIDE}/release/${ARTIFACT_NAME}"
  NESTED_ARTIFACT_PATH="${TARGET_DIR_OVERRIDE}/${BUILD_TARGET}/release/${ARTIFACT_NAME}"

  built_with_component=false

  if command -v cargo-component >/dev/null 2>&1; then
    if cargo component build --release --package "${PACKAGE_NAME}" --target "${BUILD_TARGET}" --target-dir "${TARGET_DIR_OVERRIDE}"; then
      built_with_component=true
    else
      echo "cargo-component build failed; falling back to cargo build for ${PACKAGE_NAME}." >&2
    fi
  else
    echo "cargo-component not found; falling back to cargo build for ${PACKAGE_NAME}." >&2
  fi

  if [ "${built_with_component}" = false ]; then
    cargo build --release --target "${BUILD_TARGET}" --package "${PACKAGE_NAME}" --target-dir "${TARGET_DIR_OVERRIDE}"
  fi

  if [ ! -f "${ARTIFACT_PATH}" ] && [ -f "${NESTED_ARTIFACT_PATH}" ]; then
    ARTIFACT_PATH="${NESTED_ARTIFACT_PATH}"
  fi

  if [ ! -f "${ARTIFACT_PATH}" ]; then
    echo "Expected artifact not found: ${ARTIFACT_PATH}" >&2
    exit 1
  fi

  cp "${ARTIFACT_PATH}" "${TARGET_DIR}/${PACKAGE_NAME}.wasm"
  echo "Built ${TARGET_DIR}/${PACKAGE_NAME}.wasm"
done

# Clean nested target triples produced by cargo-component to keep output tidy.
rm -rf "${TARGET_DIR_OVERRIDE}/wasm32-wasip1" "${TARGET_DIR_OVERRIDE}/wasm32-wasip2" || true
