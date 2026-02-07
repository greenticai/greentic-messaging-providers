#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT_DIR}"

if ! command -v greentic-component >/dev/null 2>&1; then
  echo "greentic-component is required for component validation" >&2
  exit 1
fi
if compgen -G "packs/*/components/*.manifest.json" >/dev/null; then
  for c in packs/*/components/*.manifest.json; do
    greentic-component doctor "$c"
  done
fi
