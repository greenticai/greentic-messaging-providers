#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
EVIDENCE_DIR="${ROOT_DIR}/docs/audit/packs/_evidence"
RG_DIR="${EVIDENCE_DIR}/rg"

mkdir -p "${RG_DIR}"

write_rg() {
  local out="$1"
  shift
  if rg -n "$@" > "${out}"; then
    return 0
  fi
  echo "no matches" > "${out}"
}

write_rg "${RG_DIR}/extensions.txt" \
  "\"greentic.provider-extension.v1|messaging.provider_ingress.v1|messaging.provider_flow_hints|messaging.subscriptions.v1|messaging.oauth.v1\"" \
  "${ROOT_DIR}/packs"

write_rg "${RG_DIR}/ops_and_exports.txt" \
  "\"ops\"|\"export\": \"handle-webhook\"|\"export\": \"sync-subscriptions\"|\"export\": \"schema-core-api\"" \
  "${ROOT_DIR}/packs"

write_rg "${RG_DIR}/public_base_url.txt" \
  "PUBLIC_BASE_URL" \
  "${ROOT_DIR}/packs" "${ROOT_DIR}/components" "${ROOT_DIR}/crates" "${ROOT_DIR}/schemas"

echo "Wrote rg evidence to ${RG_DIR}."
