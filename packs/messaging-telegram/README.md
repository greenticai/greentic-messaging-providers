# Messaging Telegram Pack

Provider-core Telegram messaging provider pack.

## Pack ID
- `messaging-telegram`

## Providers
- `messaging.telegram.bot` (capabilities: messaging; ops: send, reply)

## Components
- `ai.greentic.component-templates`
- `messaging-provider-telegram`
- `messaging-ingress-telegram`
- `templates`

## Secrets
- `TELEGRAM_BOT_TOKEN` (tenant): Telegram bot token used for sendMessage requests.

## Flows
- `diagnostics`
- `setup_custom`
- `setup_default`
- `verify_webhooks`

## Setup
Inputs:
- Config required: bot_token, public_base_url
- Config optional: default_chat_id
- Secrets required: TELEGRAM_BOT_TOKEN
- Secrets optional: none

Writes:
- Config keys: bot_token, public_base_url, default_chat_id
- Secrets: TELEGRAM_BOT_TOKEN

Webhooks:
- public_base_url + /webhooks/telegram

Subscriptions:
- none

OAuth:
- not required
