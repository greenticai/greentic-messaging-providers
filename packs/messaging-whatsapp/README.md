# Messaging WhatsApp Pack

WhatsApp messaging provider — Cloud API with webhook ingress.

## Pack ID
- `messaging-whatsapp`

## Providers
- `messaging.whatsapp.cloud` (capabilities: messaging; ops: send, reply, qa-spec, apply-answers, i18n-keys)

## Components
- `messaging-provider-whatsapp` — core provider WASM (secrets-store + http-client)
- `messaging-ingress-whatsapp` — webhook ingress WASM

## Secrets
- `WHATSAPP_TOKEN` — WhatsApp Cloud API access token
- `WHATSAPP_VERIFY_TOKEN` — webhook verification token (optional)
- `WHATSAPP_PHONE_NUMBER_ID` — phone number ID for sending

## Flows
- `setup_default` — configures provider via `messaging.configure` op
- `requirements` — validates provider configuration

## Setup
Inputs:
- Config required: phone_number_id, public_base_url
- Config optional: business_account_id
- Secrets required: WHATSAPP_TOKEN

Webhooks:
- public_base_url + /webhooks/whatsapp

## Extensions
- `greentic.ext.capabilities.v1` — capability offer `messaging-whatsapp-v1`
- `greentic.provider-extension.v1` — provider type, ops, runtime binding
- `messaging.provider_ingress.v1` — webhook ingress (supports_webhook_validation: true)
