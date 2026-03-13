#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

MODE="${1:-sync}"
if [ "${MODE}" != "sync" ] && [ "${MODE}" != "--check" ] && [ "${MODE}" != "check" ]; then
  echo "usage: $0 [sync|--check]" >&2
  exit 2
fi

if [ ! -f "${ROOT_DIR}/Cargo.lock" ]; then
  echo "Cargo.lock not found; cannot resolve greentic-interfaces version" >&2
  exit 1
fi

GREENTIC_INTERFACES_VERSION="$(
  awk '
    /^\[\[package\]\]$/ { in_pkg=1; name=""; version=""; next }
    in_pkg && /^name = / { name=$3; gsub(/"/, "", name); next }
    in_pkg && /^version = / { version=$3; gsub(/"/, "", version); next }
    in_pkg && name=="greentic-interfaces" && version != "" { print version; exit }
  ' "${ROOT_DIR}/Cargo.lock"
)"

if [ -z "${GREENTIC_INTERFACES_VERSION}" ]; then
  echo "could not resolve greentic-interfaces from Cargo.lock" >&2
  exit 1
fi

SRC_WIT_ROOT=""
find_registry_wit_root() {
  local registry_src candidate
  for registry_src in "${HOME}/.cargo/registry/src"/*; do
    [ -d "${registry_src}" ] || continue
    candidate="${registry_src}/greentic-interfaces-${GREENTIC_INTERFACES_VERSION}/wit/greentic"
    if [ -d "${candidate}" ]; then
      echo "${candidate}"
      return 0
    fi
  done
  return 1
}

SRC_WIT_ROOT="$(find_registry_wit_root || true)"
if [ -z "${SRC_WIT_ROOT}" ]; then
  echo "greentic-interfaces-${GREENTIC_INTERFACES_VERSION} WIT not found in ~/.cargo/registry/src" >&2
  echo "running 'cargo fetch --locked' to populate registry sources..." >&2
  cargo fetch --locked >&2
  SRC_WIT_ROOT="$(find_registry_wit_root || true)"
fi

if [ -z "${SRC_WIT_ROOT}" ]; then
  echo "greentic-interfaces-${GREENTIC_INTERFACES_VERSION} WIT still not found in ~/.cargo/registry/src after cargo fetch" >&2
  exit 1
fi

TARGET_FILES=()
while IFS= read -r file; do
  TARGET_FILES+=("${file}")
done < <(find "${ROOT_DIR}/components" -type f -path '*/wit/*/deps/*/package.wit' | sort)

if [ "${#TARGET_FILES[@]}" -eq 0 ]; then
  echo "no component dependency WIT files found under components/**/wit/**/deps/*/package.wit" >&2
  exit 1
fi

updated=0
checked=0
skipped=0
missing=0

find_source_file_for_package() {
  local pkg="$1"

  if command -v rg >/dev/null 2>&1; then
    rg -l -F "package ${pkg};" "${SRC_WIT_ROOT}" -g 'package.wit' | head -n1 || true
  else
    grep -R -F -l --include='package.wit' "package ${pkg};" "${SRC_WIT_ROOT}" 2>/dev/null | head -n1 || true
  fi
}

for target in "${TARGET_FILES[@]}"; do
  pkg="$(sed -n 's/^package \(.*\);$/\1/p' "${target}" | head -n1)"
  if [ -z "${pkg}" ]; then
    echo "skip: unable to parse package declaration in ${target}" >&2
    skipped=$((skipped + 1))
    continue
  fi

  source_file="$(find_source_file_for_package "${pkg}")"
  if [ -z "${source_file}" ]; then
    # Some dependency packages are local-only (for example provider:common).
    skipped=$((skipped + 1))
    continue
  fi

  checked=$((checked + 1))
  if cmp -s "${source_file}" "${target}"; then
    continue
  fi

  if [ "${MODE}" = "--check" ] || [ "${MODE}" = "check" ]; then
    echo "outdated: ${target} (source: ${source_file})" >&2
    missing=$((missing + 1))
    continue
  fi

  cp "${source_file}" "${target}"
  echo "synced ${target} <- ${source_file}"
  updated=$((updated + 1))
done

if [ "${MODE}" = "--check" ] || [ "${MODE}" = "check" ]; then
  if [ "${missing}" -gt 0 ]; then
    echo "WIT dependency drift detected: ${missing} file(s) differ from greentic-interfaces-${GREENTIC_INTERFACES_VERSION}" >&2
    exit 1
  fi
  echo "WIT dependency check passed (${checked} file(s) validated, ${skipped} skipped local-only package(s))"
  exit 0
fi

echo "WIT dependency sync complete (${updated} updated, ${checked} checked, ${skipped} skipped local-only package(s))"
