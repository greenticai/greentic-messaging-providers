# Messaging Slack Pack

Provider-core Slack messaging pack (chat.postMessage).

## Pack ID
- `messaging-slack`

## Providers
- `messaging.slack.api` (capabilities: messaging; ops: send, reply)

## Components
- `ai.greentic.component-templates`
- `messaging-provider-slack`
- `messaging-ingress-slack`
- `templates`

## Secrets
- `SLACK_BOT_TOKEN` (tenant): Slack bot token used for chat.postMessage calls.
- `SLACK_SIGNING_SECRET` (tenant): Slack signing secret (optional for future webhook validation).

## Flows
- `diagnostics`
- `rotate_credentials`
- `setup_custom`
- `setup_default`
- `verify_webhooks`

## Setup
Inputs:
- Config required: bot_token, public_base_url
- Config optional: default_channel, team_id
- Secrets required: SLACK_BOT_TOKEN
- Secrets optional: SLACK_SIGNING_SECRET

Writes:
- Config keys: bot_token, public_base_url, default_channel, team_id
- Secrets: SLACK_BOT_TOKEN, SLACK_SIGNING_SECRET

Webhooks:
- public_base_url + /webhooks/slack

Subscriptions:
- none

OAuth:
- required
