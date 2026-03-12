#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT_DIR}"

if ! command -v greentic-flow >/dev/null 2>&1; then
  echo "greentic-flow is required for flow validation" >&2
  exit 1
fi

# Step 06/07 operate on messaging packs; validating only this set keeps CI
# scoped to generated provider flows and avoids unrelated fixture packs.
shopt -s nullglob
flows=(packs/messaging-*/flows/*.ygtc)
if [ "${#flows[@]}" -eq 0 ]; then
  echo "No messaging flows found under packs/messaging-*/flows/*.ygtc"
  exit 0
fi

for f in "${flows[@]}"; do
  greentic-flow doctor "$f"
done
