# Microsoft Teams (Bot Framework) — setup

Teams bot via Bot Framework (activities POSTed to a Greentic ingress), not Graph
subscriptions. The wizard (`greentic-teams-setup-v4` stepper, backed by
`messaging-ingress-teams/src/setup.rs`) provisions everything — **including the
Azure Bot — automatically**. No manual Azure portal steps.

## Prerequisites

- An **Azure subscription** (any; a free account works) — the wizard creates the
  Azure Bot resource in it.
- Tools on PATH: `greentic-pack`, `jq`, `greentic-setup`, `greentic-start`, and the
  `wasm32-wasip2` Rust target.

## Build + run

```bash
scripts/build_provider.sh messaging-teams      # → dist/packs/messaging-teams.gtpack
greentic-setup bundle init /tmp/teams.gtbundle
greentic-setup bundle add dist/packs/messaging-teams.gtpack --bundle /tmp/teams.gtbundle --tenant demo
greentic-start start --bundle /tmp/teams.gtbundle --tenant demo --no-browser --cloudflared on
# open: http://127.0.0.1:8080/v1/web/messaging-teams/setup/demo/trial.html
```

Set `public_base_url` to the printed `…trycloudflare.com` host (Advanced config).

## The 7 steps (the wizard runs them)

1. `graph_admin_consent` — Microsoft Graph device-code admin login
2. `bot_app_identity` — create/reuse the bot's Entra app + secret
3. `microsoft_bot_channel_registration_consent` — Azure-management device-code login
4. `bot_framework_endpoint_registration` — **auto-creates the Azure Bot + enables the Teams channel** (ARM)
5. `teams_app_publish` — builds + uploads the Teams app to your tenant catalog
6. `teams_app_user_install` — installs it for you
7. `first_bot_framework_post` — completes when a real Teams message hits the ingress

You do **two device-code logins** (Graph + Azure management); the rest is automatic.
At the end click **Add to Teams** and message the bot → **7/7**.

The bot's app id (`ms_bot_app_id`) is what the host verifies each Bot Framework
token's audience against. The wizard writes it; a deployment configured without
the wizard sets the optional **Bot app ID** setup answer instead. Without it,
Teams text still flows but files from Teams are not read.

## Test (no Microsoft account)

```bash
npx playwright install chromium                       # one-time
./scripts/test_messaging_teams_setup_playwright.sh    # 15-test stepper e2e vs fake backend
```
