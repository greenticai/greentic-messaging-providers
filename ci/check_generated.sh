#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

ci/gen_flows.sh

if ! git diff --exit-code packs/; then
  echo "❌ Flows are out of sync." >&2
  echo "Run: ci/gen_flows.sh" >&2
  exit 1
fi
