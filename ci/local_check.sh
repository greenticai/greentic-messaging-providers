#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

echo "==> cargo fmt --check"
cargo fmt --check

echo "==> tools/build_components.sh"
./tools/build_components.sh

echo "==> cargo test --workspace"
cargo test --workspace

echo "All checks completed."
