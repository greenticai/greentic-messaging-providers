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

echo "==> greentic-flow doctor --validate (packs/*/flows)"
if ! command -v greentic-flow >/dev/null 2>&1; then
  echo "greentic-flow is required for flow validation" >&2
  exit 1
fi
if compgen -G "packs/*/flows/*.ygtc" >/dev/null; then
  for f in packs/*/flows/*.ygtc; do
    greentic-flow doctor "$f"
  done
fi

echo "==> greentic-component doctor --validate (components manifests)"
if ! command -v greentic-component >/dev/null 2>&1; then
  echo "greentic-component is required for component validation" >&2
  exit 1
fi
if compgen -G "packs/*/components/*.manifest.json" >/dev/null; then
  for c in packs/*/components/*.manifest.json; do
    greentic-component doctor "$c"
  done
fi

run_publish_packs="${RUN_PUBLISH_PACKS:-${CI:-0}}"
case "${run_publish_packs}" in
  1|true|TRUE|yes|YES) run_publish_packs=1 ;;
  *) run_publish_packs=0 ;;
esac

if [ "${run_publish_packs}" -eq 1 ]; then
  if ! command -v cargo-binstall >/dev/null 2>&1; then
    echo "==> Installing cargo-binstall"
    cargo install cargo-binstall --locked
  fi
  if ! command -v greentic-messaging-test >/dev/null 2>&1; then
    echo "==> Installing greentic-messaging-test"
    cargo binstall greentic-messaging-test --no-confirm --locked
  fi
  echo "==> tools/publish_packs_oci.sh (dry-run, PACK_VERSION=${PACK_VERSION})"
  DRY_RUN=1 ./tools/publish_packs_oci.sh
  if compgen -G "dist/packs/messaging-*.gtpack" >/dev/null; then
    echo "==> greentic-pack doctor --validate (dist/packs)"
    for p in dist/packs/messaging-*.gtpack; do
      greentic-pack doctor --validate --pack "$p"
    done
    echo "==> python3 tools/validate_pack_fixtures.py"
    python3 tools/validate_pack_fixtures.py
    echo "==> tools/validate_gtpack_flows.sh"
    ./tools/validate_gtpack_flows.sh
  fi
else
  echo "==> tools/publish_packs_oci.sh (dry-run; rebuild dist/packs)"
  DRY_RUN=1 PACKC_BUILD_FLAGS="--offline" ./tools/publish_packs_oci.sh
fi

echo "==> cargo test --workspace"
cargo test --workspace

echo "All checks completed."
