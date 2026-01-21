# Messaging Webchat Pack

Provider-core WebChat messaging pack (send + ingest).

## Pack ID
- `messaging-webchat`

## Providers
- `messaging.webchat` (capabilities: messaging; ops: send, ingest)

## Components
- `ai.greentic.component-templates`
- `messaging-provider-webchat`
- `templates`

## Secrets
- None.

## Flows
- `diagnostics`
- `setup_custom`
- `setup_default`
- `verify_webhooks`

## Setup
Inputs:
- Config required: mode, public_base_url
- Config optional: ingress_path
- Secrets required: none
- Secrets optional: none

Writes:
- Config keys: mode, public_base_url, ingress_path
- Secrets: none

Webhooks:
- public_base_url + /webhooks/webchat

Subscriptions:
- none

OAuth:
- not required
