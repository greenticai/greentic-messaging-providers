#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="${ROOT_DIR}/target/components"
BUILD_TARGET="wasm32-wasip2"
TARGET_DIR_OVERRIDE="${ROOT_DIR}/target/${BUILD_TARGET}"
PACKAGES=("secrets-probe" "slack" "teams" "telegram" "webchat" "webex" "whatsapp" "messaging-provider-dummy" "messaging-provider-telegram" "messaging-provider-teams" "messaging-provider-email" "messaging-provider-slack" "messaging-provider-webex" "messaging-provider-whatsapp" "messaging-provider-webchat")

if ! rustup target list --installed | grep -q "${BUILD_TARGET}"; then
  echo "Installing Rust target ${BUILD_TARGET}..."
  rustup target add "${BUILD_TARGET}"
fi

if ! command -v cargo-component >/dev/null 2>&1; then
  echo "cargo-component not found; installing..."
  cargo install cargo-component --locked
fi

mkdir -p "${TARGET_DIR}"
mkdir -p "${TARGET_DIR_OVERRIDE}"

for PACKAGE_NAME in "${PACKAGES[@]}"; do
  ARTIFACT_NAME="${PACKAGE_NAME//-/_}.wasm"
  ARTIFACT_PATH="${TARGET_DIR_OVERRIDE}/release/${ARTIFACT_NAME}"
  NESTED_ARTIFACT_PATH="${TARGET_DIR_OVERRIDE}/${BUILD_TARGET}/release/${ARTIFACT_NAME}"

  cargo component build --release --package "${PACKAGE_NAME}" --target "${BUILD_TARGET}" --target-dir "${TARGET_DIR_OVERRIDE}"

  if [ ! -f "${ARTIFACT_PATH}" ] && [ -f "${NESTED_ARTIFACT_PATH}" ]; then
    ARTIFACT_PATH="${NESTED_ARTIFACT_PATH}"
  fi

  if [ ! -f "${ARTIFACT_PATH}" ]; then
    echo "Expected artifact not found: ${ARTIFACT_PATH}" >&2
    exit 1
  fi

  cp "${ARTIFACT_PATH}" "${TARGET_DIR}/${PACKAGE_NAME}.wasm"
  if command -v wasm-tools >/dev/null 2>&1; then
    if ! wasm-tools component wit "${TARGET_DIR}/${PACKAGE_NAME}.wasm" | grep -q "wasi:cli/"; then
      echo "Artifact ${PACKAGE_NAME} does not appear to target WASI preview 2 (missing wasi:cli import)" >&2
      exit 1
    fi
  else
    echo "wasm-tools not found; skipping WASI preview 2 check for ${PACKAGE_NAME}" >&2
  fi
  echo "Built ${TARGET_DIR}/${PACKAGE_NAME}.wasm"
done

# Clean nested target triples produced by cargo-component to keep output tidy.
rm -rf "${TARGET_DIR_OVERRIDE}/wasm32-wasip1" "${TARGET_DIR_OVERRIDE}/wasm32-wasip2" || true
