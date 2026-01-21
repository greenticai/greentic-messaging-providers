# Messaging Email Pack

Provider-core SMTP email sender pack (simulated send).

## Pack ID
- `messaging-email`

## Providers
- `messaging.email.smtp` (capabilities: messaging; ops: send, reply)

## Components
- `ai.greentic.component-templates`
- `messaging-provider-email`
- `templates`

## Secrets
- `EMAIL_PASSWORD` (tenant): SMTP password secret key

## Flows
- `diagnostics`
- `setup_custom`
- `setup_default`

## Setup
Inputs:
- Config required: host, username, from_address
- Config optional: port, use_tls
- Secrets required: EMAIL_PASSWORD
- Secrets optional: none

Writes:
- Config keys: host, username, from_address, port, use_tls
- Secrets: EMAIL_PASSWORD

Webhooks:
- none

Subscriptions:
- none

OAuth:
- not required
