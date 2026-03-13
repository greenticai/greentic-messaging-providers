#!/usr/bin/env python3
from __future__ import annotations

import json
import sys
from pathlib import Path


LEGACY_HELPER_PATHS = {
    "components/ai.greentic.component-provision",
    "components/ai.greentic.component-qa",
}


def is_suppressible(diag: dict) -> bool:
    if diag.get("code") != "PACK_LOCK_COMPONENT_DESCRIBE_FAILED":
        return False
    if diag.get("path") not in LEGACY_HELPER_PATHS:
        return False
    message = str(diag.get("message", ""))
    hint = str(diag.get("hint", ""))
    return (
        "missing exported descriptor instance" in message
        and "greentic:component@0.6.0" in hint
    )


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: filter_pack_doctor_json.py <doctor-json>", file=sys.stderr)
        return 2

    raw = Path(sys.argv[1]).read_text(encoding="utf-8")
    if not raw.strip():
        print("doctor JSON was empty", file=sys.stderr)
        return 1

    try:
        data = json.loads(raw)
    except json.JSONDecodeError as exc:
        print(f"doctor JSON was invalid: {exc}", file=sys.stderr)
        return 1
    validation = data.get("validation")
    if isinstance(validation, dict):
        diagnostics = validation.get("diagnostics") or []
        filtered = [diag for diag in diagnostics if not is_suppressible(diag)]
        validation["diagnostics"] = filtered
        validation["has_errors"] = any(
            str(diag.get("severity", "")).lower() == "error" for diag in filtered
        )

    json.dump(data, sys.stdout, indent=2, sort_keys=True)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
