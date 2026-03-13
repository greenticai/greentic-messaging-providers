#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT_DIR}"

STAGE_PACK_TEMPLATES=1 "${ROOT_DIR}/ci/lib/stage_local_components.sh"
