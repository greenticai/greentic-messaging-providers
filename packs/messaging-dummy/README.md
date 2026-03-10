# Messaging Dummy Pack

Deterministic provider for CI and integration tests.

## Pack ID
- `messaging-dummy`

## Providers
- `messaging.dummy` (capabilities: messaging; ops: send, qa-spec, apply-answers, i18n-keys)

## Components
- `messaging-provider-dummy` — core provider WASM

## Secrets
- None.

## Flows
- `setup_default` — configures provider via `messaging.configure` op
- `requirements` — validates provider configuration

## Setup
Inputs:
- Config required: none
- Secrets required: none

## Extensions
- `greentic.ext.capabilities.v1` — capability offer `messaging-dummy-v1` (requires_setup: false)
- `greentic.provider-extension.v1` — provider type, ops, runtime binding
