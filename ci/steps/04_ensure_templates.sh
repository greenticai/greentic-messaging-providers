#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT_DIR}"

templates_src="${ROOT_DIR}/components/templates"
wasm_src="${templates_src}/templates.wasm"
manifest_src="${templates_src}/component.manifest.json"
if [ ! -f "${wasm_src}" ]; then
  echo "Templates component missing at ${wasm_src}" >&2
  exit 1
fi
for pack in "${ROOT_DIR}/packs"/*; do
  [ -d "${pack}" ] || continue
  dest_dir="${pack}/components/templates"
  mkdir -p "${dest_dir}"
  cp -f "${wasm_src}" "${dest_dir}/templates.wasm"
  if [ -f "${manifest_src}" ]; then
    cp -f "${manifest_src}" "${dest_dir}/component.manifest.json"
  fi
done
