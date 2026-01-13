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
