#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

rm -rf "${ROOT_DIR}/target/generated/providers"

cargo run -p greentic-messaging-packgen -- generate-all \
  --spec-dir specs/providers \
  --out target/generated/providers

for dir in "${ROOT_DIR}"/target/generated/providers/*; do
  [ -d "${dir}" ] || continue
  pack_id="$(basename "${dir}")"
  src_flows="${dir}/flows"
  dest_flows="${ROOT_DIR}/packs/${pack_id}/flows"
  if [ ! -d "${src_flows}" ]; then
    echo "No flows found for ${pack_id} under ${src_flows}" >&2
    continue
  fi
  rm -rf "${dest_flows}"
  mkdir -p "${dest_flows}"
  cp -a "${src_flows}/." "${dest_flows}/"
done
