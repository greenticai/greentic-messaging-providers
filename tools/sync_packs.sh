#!/usr/bin/env bash
set -euo pipefail

# Regenerates pack manifests, syncs schemas, bumps versions, and stages WASM artifacts from target/components.

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PACKS_DIR="${PACKS_DIR:-${ROOT_DIR}/packs}"
TARGET_COMPONENTS="${ROOT_DIR}/target/components"
VERSION="${PACK_VERSION:-}"

if [ -z "${VERSION}" ]; then
  VERSION="$(python3 - <<'PY'
from pathlib import Path
import tomllib
data = tomllib.loads(Path("Cargo.toml").read_text())
print(data.get("workspace", {}).get("package", {}).get("version", "0.0.0"))
PY
)"
fi

echo "Using version: ${VERSION}"

command -v jq >/dev/null 2>&1 || { echo "jq is required" >&2; exit 1; }
command -v python3 >/dev/null 2>&1 || { echo "python3 is required" >&2; exit 1; }

if [ ! -d "${TARGET_COMPONENTS}" ]; then
  echo "Building components..."
  "${ROOT_DIR}/tools/build_components.sh"
fi

update_pack_yaml_version() {
  local yaml_path="$1"
  [ -f "${yaml_path}" ] || return 0
  python3 - "$yaml_path" "$VERSION" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
version = sys.argv[2]
lines = path.read_text().splitlines()
updated = False
out = []
for line in lines:
    stripped = line.lstrip()
    indent = len(line) - len(stripped)
    if indent == 0 and stripped.startswith("version:"):
        prefix = line.split("version:")[0] + "version: "
        out.append(f"{prefix}{version}")
        updated = True
    else:
        out.append(line)
if not updated:
    out.append(f"version: {version}")
path.write_text("\n".join(out) + "\n")
PY
}

copy_schema() {
  local pack_dir="$1"
  local schema_path="$2"
  local src="${ROOT_DIR}/${schema_path}"
  local dest="${pack_dir}/${schema_path}"
  if [ -f "${src}" ]; then
    mkdir -p "$(dirname "${dest}")"
    cp "${src}" "${dest}"
  else
    echo "Warning: schema not found at ${src}" >&2
  fi
}

for dir in "${PACKS_DIR}"/*; do
  [ -d "${dir}" ] || continue
  if [ ! -f "${dir}/pack.manifest.json" ]; then
    echo "Skipping ${dir}: no pack.manifest.json"
    continue
  fi

  echo "Syncing $(basename "${dir}")..."
  update_pack_yaml_version "${dir}/pack.yaml"
  python3 "${ROOT_DIR}/tools/generate_pack_metadata.py" \
    --pack-dir "${dir}" \
    --components-dir "${ROOT_DIR}/components" \
    --version "${VERSION}" \
    --include-capabilities-cache

  mkdir -p "${dir}/components"
  while IFS=$'\t' read -r comp wasm_path; do
    [ -z "${comp}" ] && continue
    wasm_rel="${wasm_path:-components/${comp}.wasm}"
    wasm_file="$(basename "${wasm_rel}")"
    src="${TARGET_COMPONENTS}/${wasm_file}"
    dest="${dir}/${wasm_rel}"
    mkdir -p "$(dirname "${dest}")"
    if [ ! -f "${src}" ]; then
      echo "Missing component artifact: ${src}" >&2
      exit 1
    fi
    cp "${src}" "${dest}"
  done < <(jq -r '.components[] | if type=="string" then [.,"components/"+.+".wasm"] else [.id, .wasm // "components/"+.id+".wasm"] end | @tsv' "${dir}/pack.manifest.json")

  while IFS= read -r schema; do
    [ -z "${schema}" ] && continue
    copy_schema "${dir}" "${schema}"
  done < <(jq -r '
    [
      (.extensions["greentic.ext.provider"].inline.providers[]?.config_schema_ref // empty),
      (.config_schema.provider_config.path // empty)
    ] | flatten | .[]? ' "${dir}/pack.manifest.json")
done

echo "Pack sync complete."
