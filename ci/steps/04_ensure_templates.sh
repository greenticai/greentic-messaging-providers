#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT_DIR}"

STAGE_PACK_TEMPLATES=1 "${ROOT_DIR}/ci/lib/stage_local_components.sh"

mkdir -p "${ROOT_DIR}/target/components"
if [ -f "${ROOT_DIR}/components/templates/templates.wasm" ]; then
  cp -f "${ROOT_DIR}/components/templates/templates.wasm" "${ROOT_DIR}/target/components/templates.wasm"
  cp -f "${ROOT_DIR}/components/templates/templates.wasm" "${ROOT_DIR}/target/components/ai.greentic.component-templates.wasm"
fi
