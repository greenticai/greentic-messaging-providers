#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT_DIR}"

if ! command -v greentic-flow >/dev/null 2>&1; then
  echo "greentic-flow is required for flow validation" >&2
  exit 1
fi
if compgen -G "packs/*/flows/*.ygtc" >/dev/null; then
  for f in packs/*/flows/*.ygtc; do
    greentic-flow doctor "$f"
  done
fi
