#!/usr/bin/env bash
set -euo pipefail

NIGHTLY_PROVIDERS="${NIGHTLY_PROVIDERS:-telegram,slack,webchat}"
WORKDIR="$(mktemp -d)"
trap 'rm -rf "${WORKDIR}"' EXIT

IFS=',' read -r -a PROVIDERS <<< "${NIGHTLY_PROVIDERS}"

trim() {
  local value="$1"
  value="${value#"${value%%[![:space:]]*}"}"
  value="${value%"${value##*[![:space:]]}"}"
  printf '%s' "${value}"
}

run_with_retry() {
  local label="$1"
  shift
  if "$@"; then
    return 0
  fi
  echo "[nightly] retry once for ${label}" >&2
  "$@"
}

json_escape() {
  local value="${1//\\/\\\\}"
  value="${value//\"/\\\"}"
  value="${value//$'\n'/\\n}"
  printf '%s' "${value}"
}

write_values_file() {
  local provider="$1"
  local file="$2"
  case "${provider}" in
    slack)
      cat > "${file}" <<JSON
{
  "config": {
    "enabled": true,
    "public_base_url": "https://example.test",
    "api_base_url": "https://slack.com/api",
    "bot_token": "$(json_escape "${E2E_SLACK_BOT_TOKEN}")",
    "default_channel": "$(json_escape "${E2E_SLACK_CHANNEL}")"
  },
  "secrets": {
    "SLACK_BOT_TOKEN": "$(json_escape "${E2E_SLACK_BOT_TOKEN}")"
  },
  "http": "real"
}
JSON
      ;;
    telegram)
      cat > "${file}" <<JSON
{
  "config": {
    "enabled": true,
    "public_base_url": "https://example.test",
    "api_base_url": "https://api.telegram.org",
    "bot_token": "$(json_escape "${E2E_TELEGRAM_BOT_TOKEN}")",
    "default_chat_id": "$(json_escape "${E2E_TELEGRAM_CHAT_ID}")"
  },
  "secrets": {
    "TELEGRAM_BOT_TOKEN": "$(json_escape "${E2E_TELEGRAM_BOT_TOKEN}")"
  },
  "http": "real"
}
JSON
      ;;
    webchat)
      cat > "${file}" <<JSON
{
  "config": {
    "enabled": true,
    "public_base_url": "https://example.test",
    "mode": "local_queue",
    "route": "nightly-webchat"
  },
  "http": "mock"
}
JSON
      ;;
    *)
      echo "unsupported provider for nightly smoke: ${provider}" >&2
      return 1
      ;;
  esac
}

provider_ready() {
  local provider="$1"
  case "${provider}" in
    slack)
      [[ -n "${E2E_SLACK_BOT_TOKEN:-}" && -n "${E2E_SLACK_CHANNEL:-}" ]]
      ;;
    telegram)
      [[ -n "${E2E_TELEGRAM_BOT_TOKEN:-}" && -n "${E2E_TELEGRAM_CHAT_ID:-}" ]]
      ;;
    webchat)
      true
      ;;
    *)
      false
      ;;
  esac
}

run_provider() {
  local provider="$1"
  local values_file="${WORKDIR}/${provider}.values.json"
  write_values_file "${provider}" "${values_file}"
  local message="nightly-real smoke ${provider} $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  case "${provider}" in
    slack)
      run_with_retry "${provider}" \
        cargo run -p greentic-messaging-tester -- \
          send --provider slack --values "${values_file}" --text "${message}" \
          --to "${E2E_SLACK_CHANNEL}" --to-kind channel
      ;;
    telegram)
      run_with_retry "${provider}" \
        cargo run -p greentic-messaging-tester -- \
          send --provider telegram --values "${values_file}" --text "${message}" \
          --to "${E2E_TELEGRAM_CHAT_ID}" --to-kind user
      ;;
    webchat)
      run_with_retry "${provider}" \
        cargo run -p greentic-messaging-tester -- \
          send --provider webchat --values "${values_file}" --text "${message}"
      ;;
  esac
}

attempted=0
skipped=0
failed=0

for raw in "${PROVIDERS[@]}"; do
  provider="$(trim "${raw}")"
  [[ -z "${provider}" ]] && continue
  if ! provider_ready "${provider}"; then
    echo "[nightly] skip ${provider}: missing required env vars"
    skipped=$((skipped + 1))
    continue
  fi
  attempted=$((attempted + 1))
  if ! run_provider "${provider}"; then
    echo "[nightly] provider failed: ${provider}" >&2
    failed=$((failed + 1))
  fi
done

echo "[nightly] summary attempted=${attempted} skipped=${skipped} failed=${failed}"
if [[ "${failed}" -gt 0 ]]; then
  exit 1
fi
