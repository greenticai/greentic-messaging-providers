#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT_DIR}"
TARGET_COMPONENTS_DIR="${TARGET_COMPONENTS_DIR:-${ROOT_DIR}/target/components}"

STAGE_PACK_TEMPLATES=1 "${ROOT_DIR}/ci/lib/stage_local_components.sh"

mkdir -p "${TARGET_COMPONENTS_DIR}"
if [ -f "${ROOT_DIR}/components/templates/templates.wasm" ]; then
  cp -f "${ROOT_DIR}/components/templates/templates.wasm" "${TARGET_COMPONENTS_DIR}/templates.wasm"
  cp -f "${ROOT_DIR}/components/templates/templates.wasm" "${TARGET_COMPONENTS_DIR}/ai.greentic.component-templates.wasm"
fi
