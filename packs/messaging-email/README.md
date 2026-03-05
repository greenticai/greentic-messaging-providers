# Messaging Email Pack

Email messaging provider — Microsoft Graph API with SMTP fallback.

## Pack ID
- `messaging-email`

## Providers
- `messaging.email.smtp` (capabilities: messaging; ops: send, reply, qa-spec, apply-answers, i18n-keys)

## Components
- `messaging-provider-email` — core provider WASM (secrets-store + http-client)

## Secrets
- `FROM_ADDRESS` — sender email address
- `GRAPH_TENANT_ID` — Azure AD tenant ID
- `MS_GRAPH_CLIENT_ID` — Azure AD app client ID
- `MS_GRAPH_REFRESH_TOKEN` — OAuth refresh token (delegated permissions)

## Flows
- `setup_default` — configures provider via `messaging.configure` op
- `requirements` — validates provider configuration

## Setup
Inputs:
- Config required: from_address, graph_tenant_id, ms_graph_client_id
- Secrets required: MS_GRAPH_REFRESH_TOKEN

## Extensions
- `greentic.ext.capabilities.v1` — capability offer `messaging-email-v1`
- `greentic.provider-extension.v1` — provider type, ops, runtime binding
