#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT_DIR}"

if ! command -v greentic-pack >/dev/null 2>&1; then
  if ! command -v cargo-binstall >/dev/null 2>&1; then
    cargo install cargo-binstall --locked
  fi
  cargo binstall greentic-pack --force --no-confirm --locked || cargo install greentic-pack --force --locked
  echo "${HOME}/.cargo/bin" >> "${GITHUB_PATH:-/dev/null}" || true
fi

./ci/gen_flows.sh
