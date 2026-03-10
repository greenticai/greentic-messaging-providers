# Messaging WebChat Pack

WebChat messaging provider — Direct Line protocol with inline ingress.

## Pack ID
- `messaging-webchat`

## Providers
- `messaging.webchat` (capabilities: messaging; ops: send, ingest, ingest_http, qa-spec, apply-answers, i18n-keys)

## Components
- `messaging-provider-webchat` — core provider WASM (secrets-store + state-store, handles both egress and ingress)

## Secrets
- `jwt_signing_key` — HS256 key used to mint Direct Line JWTs

## Flows
- `setup_default` — configures provider via `messaging.configure` op
- `requirements` — validates provider configuration

## Setup
Inputs:
- Config required: `public_base_url`
- Config optional: `ingress_path`
- Secrets required: `jwt_signing_key`

## Direct Line (polling) contract

- `POST /v3/directline/tokens/generate`: mint user tokens
- `POST /v3/directline/conversations`: create conversation (requires Bearer token)
- `POST /v3/directline/conversations/{id}/activities`: send activity
- `GET /v3/directline/conversations/{id}/activities`: poll activities (watermark-based)

## Extensions
- `greentic.ext.capabilities.v1` — capability offer `messaging-webchat-v1`
- `greentic.provider-extension.v1` — provider type, ops, runtime binding
- `messaging.provider_ingress.v1` — inline ingress (same component handles webhooks)
