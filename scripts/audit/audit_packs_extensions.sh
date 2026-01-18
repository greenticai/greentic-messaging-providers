#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
EVIDENCE_DIR="${ROOT_DIR}/docs/audit/packs/_evidence"
MANIFEST_DIR="${EVIDENCE_DIR}/manifests"
EXT_DIR="${EVIDENCE_DIR}/extensions"

mkdir -p "${EXT_DIR}"

python3 - <<'PY' "${MANIFEST_DIR}" "${EXT_DIR}"
import json
import sys
from pathlib import Path

manifest_dir = Path(sys.argv[1])
ext_dir = Path(sys.argv[2])

for manifest_path in sorted(manifest_dir.glob("*.manifest.json")):
    manifest = json.loads(manifest_path.read_text())
    exts = manifest.get("extensions", {}) or {}
    data = {
        "pack": manifest.get("name"),
        "version": manifest.get("version"),
        "extensions": {},
    }
    for key, value in exts.items():
        data["extensions"][key] = value
    out_path = ext_dir / f"{manifest_path.stem.replace('.manifest','')}.extensions.json"
    out_path.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n")
PY

echo "Wrote extension summaries to ${EXT_DIR}."
