# Messaging Teams Pack

Provider-core Microsoft Teams messaging pack (Graph send).

## Pack ID
- `messaging-teams`

## Providers
- `messaging.teams.graph` (capabilities: messaging; ops: send, reply)

## Components
- `ai.greentic.component-templates`
- `messaging-provider-teams`
- `messaging-ingress-teams`
- `templates`

## Secrets
- `MS_GRAPH_CLIENT_SECRET` (tenant): Client secret used for client_credentials or refresh flows.
- `MS_GRAPH_REFRESH_TOKEN` (tenant): Refresh token used when auth_mode selects refresh_token grant.

## Flows
- `diagnostics`
- `setup_custom`
- `setup_default`
- `verify_webhooks`

## Setup
Inputs:
- Config required: tenant_id, client_id, public_base_url
- Config optional: default_channel
- Secrets required: MS_GRAPH_CLIENT_SECRET, MS_GRAPH_REFRESH_TOKEN
- Secrets optional: none

Writes:
- Config keys: tenant_id, client_id, public_base_url, default_channel
- Secrets: MS_GRAPH_CLIENT_SECRET, MS_GRAPH_REFRESH_TOKEN

Webhooks:
- public_base_url + /webhooks/teams

Subscriptions:
- required

OAuth:
- required
