#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT_DIR}"

GREENTIC_PACK_MIN_VERSION="0.4.89"
GREENTIC_PACK_VERSION="${GREENTIC_PACK_VERSION:-^0.4}"

# Ensure greentic-pack >= GREENTIC_PACK_MIN_VERSION (add-extension requires 0.4.89+)
needs_upgrade=0
if command -v greentic-pack >/dev/null 2>&1; then
  current="$(greentic-pack --version | awk '{print $NF}')"
  cur_patch="${current##0.4.}"
  min_patch="${GREENTIC_PACK_MIN_VERSION##0.4.}"
  if [ "${cur_patch}" -lt "${min_patch}" ] 2>/dev/null; then
    echo "greentic-pack ${current} is too old (need >= ${GREENTIC_PACK_MIN_VERSION}), removing stale binary..."
    rm -f "$(command -v greentic-pack)"
    needs_upgrade=1
  else
    echo "Using existing greentic-pack: greentic-pack ${current}"
  fi
else
  needs_upgrade=1
fi

if [ "${needs_upgrade}" -eq 1 ]; then
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

./ci/gen_flows.sh
