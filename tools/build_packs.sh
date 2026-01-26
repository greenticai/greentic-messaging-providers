#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

PACK_MODE="full"
if [ "${1:-}" = "--packs-only" ]; then
  PACK_MODE="packs-only"
  shift
fi

cd "${ROOT_DIR}"

if [ "${PACK_MODE}" = "full" ]; then
  ./tools/build_components.sh
  ./tools/sync_packs.sh
else
  echo "==> ci/gen_flows.sh (ensuring pack flows exist)"
  ./ci/gen_flows.sh
fi

./tools/build_packs_only.sh
