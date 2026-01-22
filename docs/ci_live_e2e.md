# Live E2E CI (manual + nightly)

Workflow: `.github/workflows/e2e-live.yml`
- Triggers: `workflow_dispatch` and nightly cron (`0 3 * * *`).
- Uses GitHub Environment `e2e-live` (configure approvals/secrets there).
- Permissions: `contents: read` only.
- Guard rails: refuses to run if `GREENTIC_ENV` is `prod`/`production`; fails if required env vars are missing.
- Packs are built locally (dry-run) and then exercised with `greentic-messaging-test packs all --glob "messaging-*.gtpack" --flow smoke` live. Secrets are injected via env-var keys and initialized with `greentic-secrets init --pack ci/secrets/<provider>.secrets-pack.yaml --non-interactive` into a file-backed secrets root (`SECRETS_ROOT`/`GREENTIC_SECRETS_DIR` in runner.temp).

Providers covered and required environment secrets (set in the `e2e-live` environment):
- Slack: `E2E_SLACK_BOT_TOKEN`, `E2E_SLACK_CHANNEL`, optional `E2E_SLACK_SIGNING_SECRET`
- Telegram: `E2E_TELEGRAM_BOT_TOKEN`, `E2E_TELEGRAM_CHAT_ID`
- Email: `E2E_SMTP_HOST`, `E2E_SMTP_PORT`, `E2E_SMTP_USERNAME`, `E2E_SMTP_PASSWORD`, `E2E_SMTP_FROM`, `E2E_SMTP_TO`
- Teams: `E2E_MS_TENANT_ID`, `E2E_MS_CLIENT_ID`, `E2E_MS_CLIENT_SECRET`, `E2E_TEAMS_CHANNEL_ID`
- Webchat: `E2E_WEBCHAT_SIGNING_SECRET`
- Webex: `E2E_WEBEX_BOT_TOKEN`, `E2E_WEBEX_ROOM_ID`
- WhatsApp: `E2E_WHATSAPP_TOKEN`, `E2E_WHATSAPP_PHONE_NUMBER_ID`, `E2E_WHATSAPP_BUSINESS_ACCOUNT_ID`, `E2E_WHATSAPP_RECIPIENT`, optional `E2E_WHATSAPP_VERIFY_TOKEN`
- Dummy: none

Secrets packs (env-backed, no real secrets):
- `ci/secrets/*.secrets-pack.yaml` map pack keys to env var names; update these if secret names change.

Local manual run (requires configured env vars):
```bash
export GREENTIC_ENV=dev GREENTIC_TENANT=ci GREENTIC_TEAM=ci
export E2E_SLACK_BOT_TOKEN=... E2E_SLACK_CHANNEL=...  # etc per provider
cargo binstall greentic-secrets --no-confirm --locked
cargo binstall greentic-messaging-test --no-confirm --locked
./tools/build_components.sh
./tools/sync_packs.sh
DRY_RUN=1 ./tools/publish_packs_oci.sh
export SECRETS_ROOT=$(mktemp -d) GREENTIC_SECRETS_DIR=$SECRETS_ROOT
for p in slack telegram email teams webchat webex whatsapp dummy; do
  greentic-secrets init --pack ci/secrets/${p}.secrets-pack.yaml --env dev --tenant ci --team ci --non-interactive
done
greentic-messaging-test packs all \
  --packs dist/packs \
  --glob 'messaging-*.gtpack' \
  --env dev --tenant ci --team ci \
  --flow smoke
```

Adding a provider:
- Add its pack id to the glob/matrix handling in `.github/workflows/e2e-live.yml`.
- Create `ci/secrets/<provider>.secrets-pack.yaml` with env var mappings.
- Document required env vars above and in the workflow guard case.
