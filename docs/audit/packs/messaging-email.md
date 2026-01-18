# Messaging Email Pack

## Pack identity
- Path: `packs/messaging-email`.
- Pack manifest version: 0.4.15 (`packs/messaging-email/pack.manifest.json:3`).
- Dist gtpack manifest version: 0.4.13 (`docs/audit/packs/_evidence/manifests/messaging-email.manifest.json:992`).

## Declared extensions
- Egress (provider extension): `greentic.provider-extension.v1` (`packs/messaging-email/pack.manifest.json:18`).
- Setup hints: `messaging.provider_flow_hints` (`packs/messaging-email/pack.manifest.json:44`).

## Entry operations per extension
- Provider ops: `send`, `reply` (`packs/messaging-email/pack.manifest.json:28`).
- Provider runtime export: `schema-core-api` (`packs/messaging-email/pack.manifest.json:36`).

## Inputs/outputs contract
- Provider contract uses `schema-core-api` with JSON byte payloads for `invoke` (`components/messaging-provider-email/wit/messaging-provider-email/deps/provider-schema-core.wit:6`).

## Config requirements (greentic-config)
- Required config keys: `host`, `username`, `from_address` (`packs/messaging-email/schemas/messaging/email/config.schema.json:36`).
- Config schema reference: `schemas/messaging/email/config.schema.json` (`packs/messaging-email/pack.manifest.json:32`).

## Secret requirements (greentic-secrets)
- Required secrets: `EMAIL_PASSWORD` (`packs/messaging-email/pack.manifest.json:56`).

## WASM components (gtpack component sources)
| component | wasm_path | digest |
| --- | --- | --- |
| diagnostics___checks | components/diagnostics___checks.wasm | sha256:5ec85da53ba2087a2990ffe996ee27702cab8123eed368f842907d112e643d00 |
| diagnostics___summary | components/diagnostics___summary.wasm | sha256:5ec85da53ba2087a2990ffe996ee27702cab8123eed368f842907d112e643d00 |
| requirements___summary | components/requirements___summary.wasm | sha256:5ec85da53ba2087a2990ffe996ee27702cab8123eed368f842907d112e643d00 |
| setup_custom___apply | components/setup_custom___apply.wasm | sha256:5ec85da53ba2087a2990ffe996ee27702cab8123eed368f842907d112e643d00 |
| setup_custom___collect | components/setup_custom___collect.wasm | sha256:5ec85da53ba2087a2990ffe996ee27702cab8123eed368f842907d112e643d00 |
| setup_custom___summary | components/setup_custom___summary.wasm | sha256:5ec85da53ba2087a2990ffe996ee27702cab8123eed368f842907d112e643d00 |
| setup_custom___validate | components/setup_custom___validate.wasm | sha256:5ec85da53ba2087a2990ffe996ee27702cab8123eed368f842907d112e643d00 |
| setup_default___apply | components/setup_default___apply.wasm | sha256:5ec85da53ba2087a2990ffe996ee27702cab8123eed368f842907d112e643d00 |
| setup_default___collect | components/setup_default___collect.wasm | sha256:5ec85da53ba2087a2990ffe996ee27702cab8123eed368f842907d112e643d00 |
| setup_default___summary | components/setup_default___summary.wasm | sha256:5ec85da53ba2087a2990ffe996ee27702cab8123eed368f842907d112e643d00 |
| setup_default___validate | components/setup_default___validate.wasm | sha256:5ec85da53ba2087a2990ffe996ee27702cab8123eed368f842907d112e643d00 |

Component sources reference: `docs/audit/packs/_evidence/manifests/messaging-email.manifest.json:432`.

Provider wasm path (no digest in pack manifest or lock):
- `components/messaging-provider-email.wasm` (`packs/messaging-email/pack.manifest.json:214`).

## PUBLIC_BASE_URL
- Not found in repo search (`docs/audit/packs/_evidence/rg/public_base_url.txt:1`).

## Subscriptions lifecycle
- No subscriptions extension declared; only provider ops are listed (`packs/messaging-email/pack.manifest.json:18`).

## Offline testability
- None stated in pack README (`packs/messaging-email/README.md:1`).

## Status
- PARTIAL: dist manifest version mismatch (0.4.13) vs pack manifest (0.4.15) (`docs/audit/packs/_evidence/manifests/messaging-email.manifest.json:992`, `packs/messaging-email/pack.manifest.json:3`).
