#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT_DIR}"
TARGET_COMPONENTS_DIR="${TARGET_COMPONENTS_DIR:-${ROOT_DIR}/target/components}"

copy_if_present() {
  local src="$1"
  local dst="$2"
  if [ -f "${src}" ]; then
    mkdir -p "$(dirname "${dst}")"
    cp -f "${src}" "${dst}"
    echo "staged ${src} -> ${dst}"
    return 0
  fi
  return 1
}

stage_core_component() {
  local name="$1"
  local target_src="${TARGET_COMPONENTS_DIR}/${name}.wasm"
  local source_dst="${ROOT_DIR}/components/${name}/${name}.wasm"
  copy_if_present "${target_src}" "${source_dst}" || true
}

stage_templates_component() {
  local staged=0
  copy_if_present \
    "${TARGET_COMPONENTS_DIR}/templates.wasm" \
    "${ROOT_DIR}/components/templates/templates.wasm" && staged=1 || true
  if [ "${staged}" -eq 0 ]; then
    copy_if_present \
      "${TARGET_COMPONENTS_DIR}/ai.greentic.component-templates.wasm" \
      "${ROOT_DIR}/components/templates/templates.wasm" || true
  fi

  if [ "${STAGE_PACK_TEMPLATES:-0}" = "1" ]; then
    local wasm_src="${ROOT_DIR}/components/templates/templates.wasm"
    local manifest_src="${ROOT_DIR}/components/templates/component.manifest.json"
    if [ ! -f "${wasm_src}" ]; then
      echo "Templates component missing at ${wasm_src}" >&2
      exit 1
    fi
    for pack in "${ROOT_DIR}/packs"/*; do
      [ -d "${pack}" ] || continue
      local dest_dir="${pack}/components/templates"
      mkdir -p "${dest_dir}"
      cp -f "${wasm_src}" "${dest_dir}/templates.wasm"
      if [ -f "${manifest_src}" ]; then
        cp -f "${manifest_src}" "${dest_dir}/component.manifest.json"
      fi
    done
  fi
}

stage_core_component provision
stage_core_component questions
stage_templates_component
