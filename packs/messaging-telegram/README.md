# Messaging Telegram Pack

Telegram messaging provider — Bot API with webhook ingress.

## Pack ID
- `messaging-telegram`

## Providers
- `messaging.telegram.bot` (capabilities: messaging; ops: send, reply, qa-spec, apply-answers, i18n-keys)

## Components
- `messaging-provider-telegram` — core provider WASM (secrets-store + http-client)
- `messaging-ingress-telegram` — webhook ingress WASM

## Secrets
- `TELEGRAM_BOT_TOKEN` — Telegram bot token from @BotFather

## Flows
- `setup_default` — configures provider via `messaging.configure` op
- `requirements` — validates provider configuration

## Setup
Inputs:
- Config required: public_base_url
- Config optional: default_chat_id
- Secrets required: TELEGRAM_BOT_TOKEN

Webhooks:
- Provide `public_base_url` as the complete callback URL

## Extensions
- `greentic.ext.capabilities.v1` — capability offer `messaging-telegram-v1`
- `greentic.provider-extension.v1` — provider type, ops, runtime binding
- `messaging.provider_ingress.v1` — webhook ingress (supports_webhook_validation: true)
