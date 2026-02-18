#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

fail=0

if ! command -v git >/dev/null 2>&1; then
  echo "git is required for WIT policy checks" >&2
  exit 1
fi

mapfile_supported=1
if ! (unset _tmp; mapfile -t _tmp < <(printf 'x\n') 2>/dev/null); then
  mapfile_supported=0
fi

if [ "${mapfile_supported}" -eq 1 ]; then
  mapfile -t WIT_FILES < <(git ls-files '*.wit')
else
  WIT_FILES=()
  while IFS= read -r f; do
    WIT_FILES+=("${f}")
  done < <(git ls-files '*.wit')
fi

echo "WIT policy check: banning greentic:component@0.4.0 and @0.5.0"
for f in "${WIT_FILES[@]}"; do
  if rg -n '^\s*package\s+greentic:component@0\.(4|5)\.0\s*;' "${f}" >/dev/null; then
    rg -nH '^\s*package\s+greentic:component@0\.(4|5)\.0\s*;' "${f}" >&2
    fail=1
  fi
done
if [ "${fail}" -ne 0 ]; then
  echo "error: forbidden legacy component package declarations found." >&2
fi

echo "WIT policy check: banning local canonical world redefinitions"
# This repository is not greentic-interfaces, so any local canonical definition is forbidden.
# Detect both single-file and split-file redefinitions at WIT package-directory scope.
for f in "${WIT_FILES[@]}"; do
  if rg -q '^\s*package\s+greentic:component@0\.6\.0\s*;' "${f}" \
    && rg -q '^\s*world\s+component-v0-v6-v0\s*\{' "${f}"; then
    echo "error: forbidden local canonical world definition: ${f}" >&2
    fail=1
  fi
done

DIRS=()
for f in "${WIT_FILES[@]}"; do
  DIRS+=("$(dirname "${f}")")
done

if [ "${#DIRS[@]}" -gt 0 ]; then
  UNIQUE_DIRS=()
  while IFS= read -r uniq_dir; do
    UNIQUE_DIRS+=("${uniq_dir}")
  done < <(printf '%s\n' "${DIRS[@]}" | sort -u)
  for dir in "${UNIQUE_DIRS[@]}"; do
    DIR_WITS=()
    while IFS= read -r wf; do
      DIR_WITS+=("${wf}")
    done < <(find "${dir}" -maxdepth 1 -type f -name '*.wit' | sort)
    if [ "${#DIR_WITS[@]}" -eq 0 ]; then
      continue
    fi

    package_file=""
    world_file=""
    for wf in "${DIR_WITS[@]}"; do
      if [ -z "${package_file}" ] \
        && rg -q '^\s*package\s+greentic:component@0\.6\.0\s*;' "${wf}"; then
        package_file="${wf}"
      fi
      if [ -z "${world_file}" ] \
        && rg -q '^\s*world\s+component-v0-v6-v0\s*\{' "${wf}"; then
        world_file="${wf}"
      fi
    done

    if [ -n "${package_file}" ] && [ -n "${world_file}" ]; then
      echo "error: forbidden split local canonical world definition in ${dir}" >&2
      echo "  package file: ${package_file}" >&2
      echo "  world file:   ${world_file}" >&2
      fail=1
    fi
  done
fi

if [ "${fail}" -ne 0 ]; then
  echo "WIT policy check failed." >&2
  exit 1
fi

echo "WIT policy check passed."
