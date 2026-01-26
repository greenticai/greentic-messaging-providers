#!/usr/bin/env python3
import json
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
COMPONENTS_DIR = ROOT / "components"


def load_json(path: Path):
    try:
        return json.loads(path.read_text())
    except json.JSONDecodeError as exc:
        raise RuntimeError(f"invalid JSON in {path}: {exc}") from exc


def resolve_ref(schema: dict, base_dir: Path) -> dict:
    ref = schema.get("$ref")
    if not ref:
        return schema
    ref_path = ref.split("#", 1)[0]
    if not ref_path:
        return schema
    ref_file = base_dir / ref_path
    if not ref_file.exists():
        raise RuntimeError(f"schema ref not found: {ref_file}")
    return load_json(ref_file)


def is_effectively_empty(schema: dict, base_dir: Path) -> bool:
    if not isinstance(schema, dict):
        return False
    if "$ref" in schema:
        schema = resolve_ref(schema, base_dir)
        return is_effectively_empty(schema, base_dir)
    if not schema:
        return True

    structural_keys = {
        "type",
        "properties",
        "required",
        "oneOf",
        "anyOf",
        "allOf",
        "enum",
        "const",
        "pattern",
        "minLength",
        "maxLength",
        "minimum",
        "maximum",
        "items",
        "additionalProperties",
    }
    meta_keys = {"$schema", "title", "description", "default", "examples"}
    keys = set(schema.keys()) - meta_keys
    if not keys.intersection(structural_keys):
        return True

    if schema.get("type") == "object":
        props = schema.get("properties")
        req = schema.get("required")
        add = schema.get("additionalProperties", True)
        if (not props) and (not req) and add in (True, None):
            return True

    return False


def main() -> int:
    manifests = sorted(COMPONENTS_DIR.rglob("component.manifest.json"))
    if not manifests:
        print("No component.manifest.json files found.", file=sys.stderr)
        return 1

    failures = []
    for manifest in manifests:
        data = load_json(manifest)
        ops = data.get("operations", [])
        base_dir = manifest.parent
        for op in ops:
            name = op.get("name", "<unknown>")
            for kind in ("input_schema", "output_schema"):
                schema = op.get(kind, {})
                if is_effectively_empty(schema, base_dir):
                    failures.append(f"{manifest}: {name} {kind} is empty or unconstrained")

    if failures:
        print("Found empty or unconstrained operation schemas:", file=sys.stderr)
        for failure in failures:
            print(f"  - {failure}", file=sys.stderr)
        return 1

    print("All component operation schemas are meaningful.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
