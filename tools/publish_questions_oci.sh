#!/usr/bin/env bash
set -euo pipefail

# Publish questions component to GHCR.

OCI_REGISTRY="${OCI_REGISTRY:-ghcr.io}"
OCI_NAMESPACE="${OCI_NAMESPACE:-greentic-ai/components}"
VERSION="${VERSION:-}"

if [ -z "${VERSION}" ]; then
  echo "VERSION is required (e.g. 0.4.20 or v0.4.20)" >&2
  exit 1
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ARTIFACT_DIR="${ROOT_DIR}/target/components"
WASM_PATH="${ARTIFACT_DIR}/questions.wasm"
MANIFEST_PATH="${ROOT_DIR}/components/questions/component.manifest.json"
README_PATH="${ROOT_DIR}/components/questions/README.md"

if ! command -v oras >/dev/null 2>&1; then
  echo "oras is required. Install it before running this script." >&2
  exit 1
fi

if [ ! -f "${WASM_PATH}" ]; then
  echo "questions.wasm not found; run tools/build_components.sh first." >&2
  exit 1
fi

ref="${OCI_REGISTRY}/${OCI_NAMESPACE}/questions:${VERSION}"
echo "Pushing ${WASM_PATH} to ${ref}"
oras_files=("questions.wasm:application/wasm")
if [ -f "${MANIFEST_PATH}" ]; then
  cp "${MANIFEST_PATH}" "${ARTIFACT_DIR}/component.manifest.json"
  oras_files+=("component.manifest.json:application/json")
fi
if [ -f "${README_PATH}" ]; then
  cp "${README_PATH}" "${ARTIFACT_DIR}/README.md"
  oras_files+=("README.md:text/markdown")
fi

(
  cd "${ARTIFACT_DIR}"
  oras push --artifact-type application/wasm-component \
    --annotation "org.opencontainers.image.title=questions" \
    --annotation "org.opencontainers.image.version=${VERSION}" \
    --annotation "org.opencontainers.image.description=CLI-first questions component" \
    "${ref}" \
    "${oras_files[@]}"
)

rm -f "${ARTIFACT_DIR}/README.md"
rm -f "${ARTIFACT_DIR}/component.manifest.json"
echo "Published ${ref}"
