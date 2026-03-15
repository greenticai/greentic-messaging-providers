#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"
GENERATED_PROVIDERS_DIR="${GENERATED_PROVIDERS_DIR:-${ROOT_DIR}/target/generated/providers}"
PACKS_DIR="${PACKS_DIR:-${ROOT_DIR}/packs}"

rm -rf "${GENERATED_PROVIDERS_DIR}"

cargo run -p greentic-messaging-packgen -- generate-all \
  --spec-dir specs/providers \
  --out "${GENERATED_PROVIDERS_DIR}"

for dir in "${GENERATED_PROVIDERS_DIR}"/*; do
  [ -d "${dir}" ] || continue
  pack_id="$(basename "${dir}")"
  src_flows="${dir}/flows"
  dest_flows="${PACKS_DIR}/${pack_id}/flows"
  if [ ! -d "${src_flows}" ]; then
    echo "No flows found for ${pack_id} under ${src_flows}" >&2
    continue
  fi
  rm -rf "${dest_flows}"
  mkdir -p "${dest_flows}"
  cp -a "${src_flows}/." "${dest_flows}/"
done
