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

  dest_root="${dir}/secret-requirements.json"
  rm -f "${dir}/assets/secret-requirements.json"
  if [ -f "${secrets_out}" ]; then
    cp "${secrets_out}" "${dest_root}"
  else
    printf '%s\n' "[]" > "${dest_root}"
  fi

  # Avoid duplicate zip entries in greentic-pack builds where this root
  # file is also materialized under assets/ by the pack tool.
  pack_yaml="${dir}/pack.yaml"
  if [ -f "${pack_yaml}" ]; then
    python3 - "${pack_yaml}" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
lines = path.read_text().splitlines()
filtered = [line for line in lines if line.strip() != "- path: secret-requirements.json"]
if filtered != lines:
    path.write_text("\n".join(filtered) + "\n")
PY
  fi
done
