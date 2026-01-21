#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PACKS_DIR="${ROOT_DIR}/packs"

for dir in "${PACKS_DIR}"/messaging-*; do
  [ -d "${dir}" ] || continue
  secrets_out="${dir}/.secret_requirements.json"
  python3 "${ROOT_DIR}/tools/generate_pack_metadata.py" \
    --pack-dir "${dir}" \
    --components-dir "${ROOT_DIR}/components" \
    --secrets-out "${secrets_out}" >/dev/null

  dest_assets="${dir}/assets/secret-requirements.json"
  dest_root="${dir}/secret-requirements.json"
  mkdir -p "$(dirname "${dest_assets}")"
  if [ -f "${secrets_out}" ]; then
    cp "${secrets_out}" "${dest_assets}"
    cp "${secrets_out}" "${dest_root}"
  else
    printf '%s\n' "[]" > "${dest_assets}"
    printf '%s\n' "[]" > "${dest_root}"
  fi
done
