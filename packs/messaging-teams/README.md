# Messaging Teams Pack

Microsoft Teams messaging provider — Bot Framework with webhook ingress.

## Pack ID
- `messaging-teams`

## Providers
- `messaging.teams.bot` (capabilities: messaging; ops: send, reply, ingest_http, render_plan, encode, send_payload, qa-spec, apply-answers, i18n-keys)

## Components
- `messaging-provider-teams` — core provider WASM (secrets-store + http-client)
- `messaging-ingress-teams` — Bot Framework webhook ingress WASM

## Secrets
- `MS_GRAPH_TENANT_ID` — Azure AD tenant ID
- `MS_GRAPH_CLIENT_ID` — Azure AD app client ID (public client)
- `MS_GRAPH_REFRESH_TOKEN` — OAuth refresh token (delegated permissions)

## Flows
- `setup_default` — configures provider via `messaging.configure` op
- `requirements` — validates provider configuration

## Setup
Inputs:
- Config required: tenant_id, client_id, public_base_url
- Config optional: default_channel
- Secrets required: MS_GRAPH_REFRESH_TOKEN

Webhooks:
- public_base_url + /webhooks/teams

## Extensions
- `greentic.ext.capabilities.v1` — capability offer `messaging-teams-v1`
- `greentic.provider-extension.v1` — provider type, ops, runtime binding
- `messaging.provider_ingress.v1` — webhook ingress (supports_webhook_validation: false)
