#!/usr/bin/env python3
"""
Compute digests for built gtpack artifacts and update packs.lock.json.

Usage:
  python3 tools/update_packs_lock.py --dist dist/packs --lock packs.lock.json
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
from datetime import datetime, timezone
from pathlib import Path


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(8192), b""):
            h.update(chunk)
    return h.hexdigest()


def load_lock(lock_path: Path) -> dict:
    if lock_path.exists():
        return json.loads(lock_path.read_text())
    return {"version": "0.0.0", "packs": []}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--dist", type=Path, default=Path("dist/packs"), help="Directory containing .gtpack artifacts")
    ap.add_argument("--lock", type=Path, default=Path("packs.lock.json"), help="Path to packs.lock.json")
    args = ap.parse_args()

    dist_dir = args.dist
    lock_path = args.lock

    if not dist_dir.exists():
        raise SystemExit(f"dist dir not found: {dist_dir}")

    lock = load_lock(lock_path)
    existing = {p["name"]: p for p in lock.get("packs", []) if "name" in p}
    gtpack_files = sorted(dist_dir.glob("*.gtpack"), key=lambda p: p.name)
    packs = []
    for gtpack in gtpack_files:
        name = gtpack.stem
        digest = sha256_file(gtpack)
        entry = dict(existing.get(name, {}))
        entry["name"] = name
        entry["file"] = str(gtpack)
        entry["digest"] = f"sha256:{digest}"
        packs.append(entry)

    lock["packs"] = packs
    lock["generated_at"] = datetime.now(timezone.utc).replace(microsecond=0).isoformat()
    try:
        lock["git_sha"] = (
            subprocess.check_output(["git", "rev-parse", "--short", "HEAD"], text=True)
            .strip()
        )
    except Exception:
        pass

    lock_path.write_text(json.dumps(lock, indent=2) + "\n")
    print(f"Updated {lock_path} with {len(packs)} pack entries.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
