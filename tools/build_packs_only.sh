#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PACK_VERSION="${PACK_VERSION:-}"
if [ -z "${PACK_VERSION}" ]; then
  if ! command -v python3 >/dev/null 2>&1; then
    echo "python3 is required to determine PACK_VERSION" >&2
    exit 1
  fi
  PACK_VERSION="$(python3 - <<'PY'
from pathlib import Path
import tomllib

data = tomllib.loads(Path("Cargo.toml").read_text())
print(data.get("workspace", {}).get("package", {}).get("version", "0.0.0"))
PY
)"
fi
PACK_VERSION="${PACK_VERSION:-${GITHUB_REF_NAME:-0.0.0}}"
PACK_VERSION="${PACK_VERSION#v}"
PACKC_BUILD_FLAGS="${PACKC_BUILD_FLAGS:-}"
DRY_RUN="${DRY_RUN:-1}"

cd "${ROOT_DIR}"
DRY_RUN="${DRY_RUN}" PACK_VERSION="${PACK_VERSION}" PACKC_BUILD_FLAGS="${PACKC_BUILD_FLAGS}" ./tools/publish_packs_oci.sh
