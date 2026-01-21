# Messaging Whatsapp Pack

Provider-core WhatsApp Cloud messaging pack.

## Pack ID
- `messaging-whatsapp`

## Providers
- `messaging.whatsapp.cloud` (capabilities: messaging; ops: send, reply)

## Components
- `ai.greentic.component-templates`
- `messaging-provider-whatsapp`
- `messaging-ingress-whatsapp`
- `templates`

## Secrets
- `WHATSAPP_TOKEN` (tenant): WhatsApp Cloud API access token.
- `WHATSAPP_VERIFY_TOKEN` (tenant): Verify token used for WhatsApp webhook validation (if configured).

## Flows
- `diagnostics`
- `setup_custom`
- `setup_default`
- `verify_webhooks`

## Setup
Inputs:
- Config required: access_token, phone_number_id, public_base_url
- Config optional: business_account_id
- Secrets required: WHATSAPP_TOKEN
- Secrets optional: WHATSAPP_VERIFY_TOKEN

Writes:
- Config keys: access_token, phone_number_id, public_base_url, business_account_id
- Secrets: WHATSAPP_TOKEN, WHATSAPP_VERIFY_TOKEN

Webhooks:
- public_base_url + /webhooks/whatsapp

Subscriptions:
- none

OAuth:
- not required
