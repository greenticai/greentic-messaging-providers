#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT_DIR}"

if ! command -v greentic-component >/dev/null 2>&1; then
  echo "greentic-component is required for component tests" >&2
  exit 1
fi

is_world_mismatch_error() {
  local log_file="$1"
  rg -q \
    "no exported instance named \`greentic:component/node@0\\.6\\.0\`|Unsupported world .*Supported: greentic:component/component@0\\.6\\.0" \
    "${log_file}"
}

tmpdir="$(mktemp -d)"
trap 'rm -rf "${tmpdir}"' EXIT
questions_wasm="${ROOT_DIR}/components/questions/questions.wasm"
if [ ! -f "${questions_wasm}" ]; then
  if [ -f "${ROOT_DIR}/target/components/questions.wasm" ]; then
    questions_wasm="${ROOT_DIR}/target/components/questions.wasm"
  else
    if [ -x "${ROOT_DIR}/tools/build_components/questions.sh" ]; then
      bash "${ROOT_DIR}/tools/build_components/questions.sh"
    fi
    if [ -f "${ROOT_DIR}/target/components/questions.wasm" ]; then
      questions_wasm="${ROOT_DIR}/target/components/questions.wasm"
    else
      echo "Questions component wasm missing; build it with tools/build_components/questions.sh" >&2
      exit 1
    fi
  fi
fi
cp "${questions_wasm}" "${tmpdir}/questions.wasm"
cp components/questions/component.manifest.json "${tmpdir}/component.manifest.json"
cp packs/messaging-dummy/assets/setup.yaml "${tmpdir}/setup.yaml"

emit_log="${tmpdir}/emit.log"
set +e
(
  cd "${tmpdir}"
  greentic-component test \
    --wasm questions.wasm \
    --manifest component.manifest.json \
    --op emit \
    --input-json '{"id":"dummy-setup","spec_ref":"assets/setup.yaml","context":{"tenant_id":"t1","env":"dev"}}' \
    --pretty
) >"${emit_log}" 2>&1
emit_rc=$?
set -e
cat "${emit_log}"
if [ "${emit_rc}" -ne 0 ]; then
  if is_world_mismatch_error "${emit_log}"; then
    echo "questions component world/export is incompatible with installed greentic-component test runner." >&2
    echo "expected by runner: greentic:component/component@0.6.0; exported by wasm: greentic:component/node@0.5.0." >&2
  fi
  exit "${emit_rc}"
fi

python3 - <<'PY' "${tmpdir}"
import json
import sys
from pathlib import Path

tmpdir = Path(sys.argv[1])
spec = {
    "provider_id": "dummy",
    "version": 1,
    "title": "Dummy provider setup",
    "questions": [],
    "id": "dummy-setup",
}
payload = {
    "id": "dummy-setup",
    "spec_json": json.dumps(spec),
    "answers_json": json.dumps({}),
}
(tmpdir / "validate_input.json").write_text(json.dumps(payload))
PY

validate_log="${tmpdir}/validate.log"
set +e
(
  cd "${tmpdir}"
  greentic-component test \
    --wasm questions.wasm \
    --manifest component.manifest.json \
    --op validate \
    --input validate_input.json \
    --pretty
) >"${validate_log}" 2>&1
validate_rc=$?
set -e
cat "${validate_log}"
if [ "${validate_rc}" -ne 0 ]; then
  if is_world_mismatch_error "${validate_log}"; then
    echo "questions component world/export is incompatible with installed greentic-component test runner." >&2
    echo "expected by runner: greentic:component/component@0.6.0; exported by wasm: greentic:component/node@0.5.0." >&2
  fi
  exit "${validate_rc}"
fi

trap - EXIT
rm -rf "${tmpdir}"
