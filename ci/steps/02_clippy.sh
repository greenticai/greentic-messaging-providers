#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT_DIR}"

TARGET_DIR="${CARGO_TARGET_DIR:-${ROOT_DIR}/target}"

clear_greentic_interfaces_wasmtime_build_cache() {
  local path
  for path in \
    "${TARGET_DIR}/debug/build" \
    "${TARGET_DIR}/debug/.fingerprint" \
    "${TARGET_DIR}/debug/deps"
  do
    [ -d "${path}" ] || continue
    find "${path}" -maxdepth 1 -mindepth 1 \
      \( -name 'greentic-interfaces-wasmtime-*' -o -name 'libgreentic_interfaces_wasmtime*' \) \
      -exec rm -rf {} +
  done
}

# Clippy can restore stale generated bindgen output from the cache that points at
# registry-local WIT staging paths which do not exist yet on a fresh runner.
bash "${ROOT_DIR}/tools/sync_wit_deps_from_greentic_interfaces.sh"
clear_greentic_interfaces_wasmtime_build_cache

if ! cargo clippy --workspace --all-targets; then
  if command -v rustup >/dev/null 2>&1; then
    toolchain="$(rustup show active-toolchain | awk '{print $1}')"
    rustup component add --toolchain "${toolchain}" clippy
    clear_greentic_interfaces_wasmtime_build_cache
    cargo clippy --workspace --all-targets
  else
    exit 1
  fi
fi
