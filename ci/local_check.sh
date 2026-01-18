#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

PACK_VERSION="${PACK_VERSION:-$(python3 - <<'PY'
from pathlib import Path
import tomllib
data = tomllib.loads(Path("Cargo.toml").read_text())
print(data.get("workspace", {}).get("package", {}).get("version", "0.0.0"))
PY
)}"
export PACK_VERSION

echo "==> cargo fmt --check"
cargo fmt --check

echo "==> tools/build_components.sh"
./tools/build_components.sh

echo "==> tools/sync_packs.sh (PACK_VERSION=${PACK_VERSION})"
./tools/sync_packs.sh

run_publish_packs="${RUN_PUBLISH_PACKS:-${CI:-0}}"
case "${run_publish_packs}" in
  1|true|TRUE|yes|YES) run_publish_packs=1 ;;
  *) run_publish_packs=0 ;;
esac

if [ "${run_publish_packs}" -eq 1 ]; then
  echo "==> tools/publish_packs_oci.sh (dry-run, PACK_VERSION=${PACK_VERSION})"
  DRY_RUN=1 ./tools/publish_packs_oci.sh
else
  echo "==> tools/publish_packs_oci.sh (skipped; set RUN_PUBLISH_PACKS=1 to enable)"
fi

echo "==> cargo test --workspace"
cargo test --workspace

echo "All checks completed."
