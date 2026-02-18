#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

echo "[registry-fixtures] regenerating fixtures from provider components"
cargo test -p provider-tests --test registry_fixtures regenerate_registry_fixtures -- --ignored

echo "[registry-fixtures] verifying stability/validity"
cargo test -p provider-tests --test registry_fixtures registry_fixtures_are_stable_and_valid

echo "[registry-fixtures] done"
