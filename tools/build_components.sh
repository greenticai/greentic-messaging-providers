#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PACKAGES=("provision" "questions" "secrets-probe" "slack" "teams" "telegram" "telegram-webhook" "webchat" "webex" "webex-webhook" "whatsapp" "messaging-ingress-slack" "messaging-ingress-teams" "messaging-ingress-telegram" "messaging-ingress-whatsapp" "messaging-provider-dummy" "messaging-provider-telegram" "messaging-provider-teams" "messaging-provider-email" "messaging-provider-slack" "messaging-provider-webex" "messaging-provider-whatsapp" "messaging-provider-webchat")

for package in "${PACKAGES[@]}"; do
  bash "${ROOT_DIR}/tools/build_components/${package}.sh"
done

# Note: do not delete nested target triples here. This script is invoked from
# multiple test binaries in parallel, and deleting shared target directories can
# race with active builds (leading to missing .fingerprint files).
