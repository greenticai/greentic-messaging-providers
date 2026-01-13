#!/usr/bin/env bash
set -euo pipefail

# Publishes built component WASM artifacts to an OCI registry using oras.
# Inputs (env):
#   OCI_REGISTRY   - e.g. ghcr.io
#   OCI_NAMESPACE  - e.g. my-org/greentic-messaging-providers
#   VERSION        - tag used for the artifact (e.g. v0.1.0)
#
# Expects artifacts at target/components/<name>.wasm (built beforehand).

if [[ -z "${OCI_REGISTRY:-}" || -z "${OCI_NAMESPACE:-}" || -z "${VERSION:-}" ]]; then
  echo "OCI_REGISTRY, OCI_NAMESPACE, and VERSION must be set in the environment." >&2
  exit 1
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ARTIFACT_DIR="${ROOT_DIR}/target/components"
LOCKFILE="${ROOT_DIR}/components.lock.json"
mkdir -p "${ARTIFACT_DIR}"

if ! command -v oras >/dev/null 2>&1; then
  echo "oras is required. Install it before running this script." >&2
  exit 1
fi

components_json="[]"

for wasm in "${ARTIFACT_DIR}"/*.wasm; do
  [ -e "$wasm" ] || { echo "No wasm artifacts found in ${ARTIFACT_DIR}"; exit 1; }
  name="$(basename "${wasm}" .wasm)"
  ref="${OCI_REGISTRY}/${OCI_NAMESPACE}/${name}:${VERSION}"

  echo "Pushing ${wasm} to ${ref}"
  # Capture digest from oras output without leaking credentials.
  digest="$(
    (
      cd "${ARTIFACT_DIR}"
      oras push --artifact-type application/wasm-component \
        "${ref}" \
        "${name}.wasm:application/wasm"
    ) | awk '/Digest:/{print $2}' | tail -n1
  )"

  if [[ -z "${digest}" ]]; then
    echo "Failed to capture digest for ${ref}" >&2
    exit 1
  fi

  component_entry=$(jq -n \
    --arg name "${name}" \
    --arg reference "${ref}" \
    --arg digest "${digest}" \
    '{name:$name, reference:$reference, digest:$digest}')

  components_json=$(jq -n \
    --argjson arr "${components_json}" \
    --argjson comp "${component_entry}" \
    '$arr + [$comp]')
done

jq -n --arg version "${VERSION}" --arg registry "${OCI_REGISTRY}" --arg namespace "${OCI_NAMESPACE}" \
  --argjson components "${components_json}" \
  '{version:$version, registry:$registry, namespace:$namespace, components:$components}' \
  > "${LOCKFILE}"

echo "Wrote lockfile to ${LOCKFILE}"
