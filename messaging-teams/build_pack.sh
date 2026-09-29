#!/usr/bin/env bash

set -euo pipefail

SOURCE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SOURCE_DIR}/.." && pwd)"
ANSWER_PATH="${SOURCE_DIR}/build-answer.json"

if ! command -v greentic-pack >/dev/null 2>&1; then
  echo "greentic-pack not found" >&2
  exit 127
fi

if ! command -v jq >/dev/null 2>&1; then
  echo "jq not found" >&2
  exit 127
fi

generated_rel="$(jq -r '.source_layout.generated_pack_dir' "${ANSWER_PATH}")"
if [ -z "${generated_rel}" ] || [ "${generated_rel}" = "null" ]; then
  echo "missing source_layout.generated_pack_dir in ${ANSWER_PATH}" >&2
  exit 1
fi

GENERATED_DIR="${ROOT_DIR}/${generated_rel}"
TMP_DIR="${TMPDIR:-/tmp}/greentic-messaging-teams-build"

rm -rf "${TMP_DIR}" "${GENERATED_DIR}"
mkdir -p "${TMP_DIR}" "$(dirname "${GENERATED_DIR}")"

while IFS= read -r build_script; do
  [ -n "${build_script}" ] || continue
  "${ROOT_DIR}/${build_script}" >/dev/null
done < <(jq -r '.runtime_components[]?.build_script // empty' "${ANSWER_PATH}")

jq '.pack_create' "${ANSWER_PATH}" > "${TMP_DIR}/pack-create-answers.json"
(
  cd "${ROOT_DIR}"
  greentic-pack wizard apply --answers "${TMP_DIR}/pack-create-answers.json" >/dev/null
)

python3 - "${SOURCE_DIR}" "${GENERATED_DIR}" <<'PY'
import json
import shutil
import sys
from pathlib import Path

source_dir = Path(sys.argv[1])
generated_dir = Path(sys.argv[2])
answer = json.loads((source_dir / "build-answer.json").read_text(encoding="utf-8"))

for rel in answer.get("assets", []):
    src = source_dir / rel
    dest = generated_dir / rel
    dest.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(src, dest)

for component in answer.get("runtime_components", []):
    src = source_dir.parent / component.get("source", "")
    dest = generated_dir / component.get("target", "")
    if not src.exists():
        raise SystemExit(f"runtime component missing: {src}")
    dest.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(src, dest)
    component_target = component.get("component_target")
    if component_target:
        component_dest = generated_dir / component_target
        component_dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(src, component_dest)
    manifest_source = component.get("manifest_source")
    manifest_target = component.get("manifest_target")
    if manifest_target:
        dest_manifest = generated_dir / manifest_target
        dest_manifest.parent.mkdir(parents=True, exist_ok=True)
        pack_manifest = {
            "id": component["id"],
            "version": component.get("version", "0.1.0"),
            "supports": ["messaging"],
            "world": component.get("world", "root:component/root"),
            "profiles": {"default": "default", "supported": ["default"]},
            "capabilities": {
                "wasi": {},
                "host": {"state": {"read": True, "write": True}},
            },
            "operations": [],
            "resources": {},
            "dev_flows": {},
        }
        dest_manifest.write_text(json.dumps(pack_manifest, indent=2) + "\n", encoding="utf-8")

# Bundle the provider config schema referenced by greentic.provider-extension.v1.
schema_src = source_dir.parent / "components/messaging-provider-teams-graph/schemas/messaging/teams/config.schema.json"
if schema_src.exists():
    schema_dest = generated_dir / "schemas/messaging/teams/public.config.schema.json"
    schema_dest.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(schema_src, schema_dest)

for entry in (answer.get("pack_overlay") or {}).get("files", []):
    rel = entry.get("path")
    content = entry.get("content", "")
    if not rel:
        continue
    dest = generated_dir / rel
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_text(content, encoding="utf-8")

pack_yaml = generated_dir / "pack.yaml"
text = pack_yaml.read_text(encoding="utf-8")
pack_version = answer.get("pack_version")
if pack_version:
    text = text.replace("version: 0.1.0", f"version: {pack_version}", 1)
# Static route assets are packaged through greentic.static-routes.v1 below.
# Do not also declare them under `assets:` because greentic-pack stages those
# entries below `assets/`, which would duplicate `assets/setup` as
# `assets/assets/setup` in the .gtpack archive.

component_lines = "components:\n"
for component in answer.get("runtime_components", []):
    component_lines += (
        f"- id: {component['id']}\n"
        f"  version: {component.get('version', answer.get('pack_version', '0.1.0'))}\n"
        f"  world: {component.get('world', 'root:component/root')}\n"
        "  supports:\n"
        "  - messaging\n"
        "  profiles:\n"
        "    default: default\n"
        "    supported:\n"
        "    - default\n"
        "  capabilities:\n"
        "    wasi:\n"
        "      random: false\n"
        "      clocks: false\n"
        "    host: {}\n"
        f"  wasm: {component['target']}\n"
    )
if answer.get("runtime_components") and "components: []" in text:
    text = text.replace("components: []", component_lines.rstrip())

setup_api = answer.get("setup_api") or {}
web_component = setup_api.get("web_component") or {}
web_component_inline = json.dumps(web_component, indent=6).replace("\n", "\n      ")
backend_contract = setup_api.get("backend_contract") or {}
backend_contract_inline = json.dumps(backend_contract, indent=6).replace("\n", "\n      ")
setup_actions = setup_api.get("actions") or {
    "schema_id": "greentic.setup.actions.v1",
    "provider_id": "messaging-teams",
    "actions": [
        {
            "id": "add-to-teams",
            "label": "Add to Teams",
            "kind": "deep_link",
            "url_template": "{add_to_teams_url}",
            "style": "primary",
            "opens_new_window": True,
            "copyable": True,
            "requires": ["add_to_teams_url"],
        }
    ],
}
setup_actions_inline = json.dumps(setup_actions, indent=6).replace("\n", "\n      ")
asset_base_path = web_component.get("asset_base_path", "/v1/web/messaging-teams/setup/{tenant}")
pack_version = answer.get("pack_version", "0.1.0")
capabilities = {
    "schema_version": 1,
    "offers": [
        {
            "cap_id": "greentic.cap.messaging.provider.v1",
            "offer_id": "messaging-teams-v1",
            "priority": 100,
            "version": pack_version,
            "provider": {
                "component_ref": "messaging-ingress-teams",
                "op": "messaging.configure",
            },
            "requires_setup": True,
            "setup": {
                "qa_ref": "components/messaging-ingress-teams",
                "web_component_ref": "greentic.setup.web-component.v1",
            },
        }
    ],
}
capabilities_inline = json.dumps(capabilities, indent=6).replace("\n", "\n      ")
extensions = f"""
extensions:
  greentic.ext.capabilities.v1:
    kind: greentic.ext.capabilities.v1
    version: "{pack_version}"
    inline: {capabilities_inline}
  greentic.static-routes.v1:
    kind: greentic.static-routes.v1
    version: "1"
    inline:
      version: 1
      routes:
      - id: messaging-teams-setup
        public_path: {asset_base_path}
        source_root: assets/setup
        index_file: README.md
        scope:
          tenant: true
          team: false
  greentic.setup.web-component.v1:
    kind: greentic.setup.web-component.v1
    version: "1"
    inline: {web_component_inline}
  greentic.setup.backend-contract.v1:
    kind: greentic.setup.backend-contract.v1
    version: "1"
    inline: {backend_contract_inline}
  greentic.setup.actions.v1:
    kind: greentic.setup.actions.v1
    version: "1"
    inline: {setup_actions_inline}
  messaging.provider_ingress.v1:
    kind: messaging.provider_ingress.v1
    version: "1"
    inline:
      component_ref: messaging-ingress-teams
      export: handle-webhook
      world: greentic:messaging-ingress-teams/messaging-ingress-teams@0.0.1
      capabilities:
        content_types:
        - application/json
        supports_webhook_validation: true
  greentic.provider-extension.v1:
    kind: greentic.provider-extension.v1
    version: "{pack_version}"
    inline:
      providers:
      - provider_type: messaging.teams
        capabilities:
        - messaging
        ops:
        - send
        - reply
        - render_plan
        - encode
        - send_payload
        - send_typing
        - qa-spec
        - apply-answers
        - i18n-keys
        config_schema_ref: schemas/messaging/teams/public.config.schema.json
        runtime:
          component_ref: messaging-provider-teams-graph
          export: schema-core-api
          world: greentic:provider/schema-core@1.0.0
  greentic.http-routes.v1:
    kind: greentic.http-routes.v1
    version: "1"
    inline:
      schema_version: 1
      routes:
      - id: messaging-teams-setup-state
        pattern: /v1/messaging/setup/messaging-teams/{{tenant}}
        methods:
        - GET
        - POST
        provider_op: ingest_http
        domain: messaging
      - id: messaging-teams-setup-actions
        pattern: /v1/messaging/setup/messaging-teams/{{tenant}}/{{action*}}
        methods:
        - GET
        - POST
        provider_op: ingest_http
        domain: messaging
      - id: messaging-teams-bot-framework-registration
        pattern: /v1/setup/messaging-teams/{{tenant}}/{{team}}/bot-framework-registration
        methods:
        - POST
        setup_component_ref: messaging-ingress-teams
        setup_op: bot-framework-registration
        domain: messaging
      - id: messaging-teams-app-publish
        pattern: /v1/setup/messaging-teams/{{tenant}}/{{team}}/teams-app-publish
        methods:
        - POST
        setup_component_ref: messaging-ingress-teams
        setup_op: teams-app-publish
        domain: messaging
      - id: messaging-teams-app-install
        pattern: /v1/setup/messaging-teams/{{tenant}}/{{team}}/teams-app-install
        methods:
        - POST
        setup_component_ref: messaging-ingress-teams
        setup_op: teams-app-install
        domain: messaging
"""
if "\nextensions:\n" not in text and not text.rstrip().endswith("extensions:"):
    text = text.rstrip() + "\n" + extensions
pack_yaml.write_text(text, encoding="utf-8")
PY

jq '.pack' "${ANSWER_PATH}" > "${TMP_DIR}/pack-update-answers.json"
(
  cd "${GENERATED_DIR}"
  greentic-pack build --in . --allow-pack-schema >/dev/null
)

echo "generated ${GENERATED_DIR}"
