#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT_DIR}"

GREENTIC_PACK_VERSION="${GREENTIC_PACK_VERSION:-^0.4}"
if command -v greentic-pack >/dev/null 2>&1; then
  echo "Using existing greentic-pack: $(greentic-pack --version)"
else
  if ! command -v cargo-binstall >/dev/null 2>&1; then
    cargo install cargo-binstall --locked
  fi
  cargo binstall greentic-pack --version "${GREENTIC_PACK_VERSION}" --force --no-confirm --locked || \
    cargo install greentic-pack --version "${GREENTIC_PACK_VERSION}" --force --locked
fi
echo "${HOME}/.cargo/bin" >> "${GITHUB_PATH:-/dev/null}" || true
greentic-pack --version

if ! command -v greentic-flow >/dev/null 2>&1; then
  if ! command -v cargo-binstall >/dev/null 2>&1; then
    cargo install cargo-binstall --locked
  fi
  cargo binstall greentic-flow --force --no-confirm --locked || cargo install greentic-flow --force --locked
  echo "${HOME}/.cargo/bin" >> "${GITHUB_PATH:-/dev/null}" || true
fi

"${ROOT_DIR}/ci/lib/stage_local_components.sh"

./ci/gen_flows.sh
