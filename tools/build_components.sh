#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEFAULT_PACKAGES=("questions" "secrets-probe" "slack" "teams" "telegram" "telegram-webhook" "webchat" "webex" "webex-webhook" "whatsapp" "messaging-ingress-slack" "messaging-ingress-teams" "messaging-ingress-telegram" "messaging-ingress-whatsapp" "messaging-provider-dummy" "messaging-provider-telegram" "messaging-provider-teams-graph" "messaging-provider-email" "messaging-provider-slack" "messaging-provider-webex" "messaging-provider-whatsapp" "messaging-provider-webchat" "messaging-provider-webchat-gui" "messaging-provider-3aigent-gui" "messaging-provider-api")

if [ -n "${COMPONENT_FILTER:-}" ]; then
  PACKAGES=()
  while IFS= read -r package; do
    [ -n "${package}" ] || continue
    PACKAGES+=("${package}")
  done < <(python3 - <<'PY' "${COMPONENT_FILTER}"
import sys

raw = sys.argv[1]
items = [part.strip() for chunk in raw.split(",") for part in chunk.split() if part.strip()]
for item in dict.fromkeys(items):
    print(item)
PY
)
else
  PACKAGES=("${DEFAULT_PACKAGES[@]}")
fi

bash "${ROOT_DIR}/tools/sync_wit_deps_from_greentic_interfaces.sh"

for package in "${PACKAGES[@]}"; do
  bash "${ROOT_DIR}/tools/build_components/${package}.sh"
done

# Note: do not delete nested target triples here. This script is invoked from
# multiple test binaries in parallel, and deleting shared target directories can
# race with active builds (leading to missing .fingerprint files).
