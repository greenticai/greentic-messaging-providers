# Messaging Dummy Pack

## Pack identity
- Path: `packs/messaging-dummy`.
- Pack manifest version: 0.4.15 (`packs/messaging-dummy/pack.manifest.json:3`).
- Dist gtpack manifest version: 0.4.13 (`docs/audit/packs/_evidence/manifests/messaging-dummy.manifest.json:128`).

## Declared extensions
- Egress (provider extension): `greentic.provider-extension.v1` (`packs/messaging-dummy/pack.manifest.json:17`).
- Setup hints: `messaging.provider_flow_hints` (`packs/messaging-dummy/pack.manifest.json:42`).

## Entry operations per extension
- Provider ops: `send` (`packs/messaging-dummy/pack.manifest.json:27`).
- Provider runtime export: `schema-core-api` (`packs/messaging-dummy/pack.manifest.json:34`).

## Inputs/outputs contract
- Provider contract uses `schema-core-api` with JSON byte payloads for `invoke` (`components/messaging-provider-dummy/wit/messaging-provider-dummy/world.wit:6`).

## Config requirements (greentic-config)
- No required keys declared (schema allows any properties) (`packs/messaging-dummy/schemas/messaging/dummy/config.schema.json:1`).
- Config schema reference: `schemas/messaging/dummy/config.schema.json` (`packs/messaging-dummy/pack.manifest.json:30`).

## Secret requirements (greentic-secrets)
- No declared secrets (`packs/messaging-dummy/pack.manifest.json:54`).

## WASM components (gtpack component sources)
| component | wasm_path | digest |
| --- | --- | --- |
| diagnostics___summary | components/diagnostics___summary.wasm | sha256:5ec85da53ba2087a2990ffe996ee27702cab8123eed368f842907d112e643d00 |
| requirements___summary | components/requirements___summary.wasm | sha256:5ec85da53ba2087a2990ffe996ee27702cab8123eed368f842907d112e643d00 |
| setup_custom___summarize | components/setup_custom___summarize.wasm | sha256:5ec85da53ba2087a2990ffe996ee27702cab8123eed368f842907d112e643d00 |
| setup_default___summarize | components/setup_default___summarize.wasm | sha256:5ec85da53ba2087a2990ffe996ee27702cab8123eed368f842907d112e643d00 |

Component sources reference: `docs/audit/packs/_evidence/manifests/messaging-dummy.manifest.json:48`.

Provider wasm path (no digest in pack manifest or lock):
- `components/messaging-provider-dummy.wasm` (`packs/messaging-dummy/pack.manifest.json:118`).

## PUBLIC_BASE_URL
- Not found in repo search (`docs/audit/packs/_evidence/rg/public_base_url.txt:1`).

## Subscriptions lifecycle
- No subscriptions extension declared; only provider ops are listed (`packs/messaging-dummy/pack.manifest.json:17`).

## Offline testability
- Deterministic provider for CI/integration tests (`packs/messaging-dummy/README.md:3`).

## Status
- PARTIAL: dist manifest version mismatch (0.4.13) vs pack manifest (0.4.15) (`docs/audit/packs/_evidence/manifests/messaging-dummy.manifest.json:128`, `packs/messaging-dummy/pack.manifest.json:3`).
