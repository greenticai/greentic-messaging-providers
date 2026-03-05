# Messaging Webex Pack

Webex messaging provider — Bot API with Adaptive Cards.

## Pack ID
- `messaging-webex`

## Providers
- `messaging.webex.bot` (capabilities: messaging; ops: send, reply, qa-spec, apply-answers, i18n-keys)

## Components
- `messaging-provider-webex` — core provider WASM (secrets-store + http-client)

## Secrets
- `WEBEX_BOT_TOKEN` — Webex bot access token

## Flows
- `setup_default` — configures provider via `messaging.configure` op
- `requirements` — validates provider configuration

## Setup
Inputs:
- Config required: public_base_url
- Config optional: default_room_id
- Secrets required: WEBEX_BOT_TOKEN

Webhooks:
- public_base_url (registered for `messages.created`)

## Extensions
- `greentic.ext.capabilities.v1` — capability offer `messaging-webex-v1`
- `greentic.provider-extension.v1` — provider type, ops, runtime binding
