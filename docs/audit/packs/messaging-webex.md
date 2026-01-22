# Messaging Webex Pack

## Pack identity
- Path: `packs/messaging-webex`.
- Pack manifest version: 0.4.15 (`packs/messaging-webex/pack.manifest.json:3`).
- Dist gtpack manifest version: 0.4.13 (`docs/audit/packs/_evidence/manifests/messaging-webex.manifest.json:1223`).

## Declared extensions
- Egress (provider extension): `greentic.provider-extension.v1` (`packs/messaging-webex/pack.manifest.json:18`).
- Setup hints: `messaging.provider_flow_hints` (`packs/messaging-webex/pack.manifest.json:44`).

## Entry operations per extension
- Provider ops: `send`, `reply` (`packs/messaging-webex/pack.manifest.json:28`).
- Provider runtime export: `schema-core-api` (`packs/messaging-webex/pack.manifest.json:36`).

## Inputs/outputs contract
- Provider contract uses `schema-core-api` with JSON byte payloads for `invoke` (`components/messaging-provider-webex/wit/messaging-provider-webex/deps/provider-schema-core/package.wit:6`).

## Config requirements (greentic-config)
- Required config keys: `public_base_url` (`packs/messaging-webex/schemas/messaging/webex/config.schema.json:19`).
- Config schema reference: `schemas/messaging/webex/config.schema.json` (`packs/messaging-webex/pack.manifest.json:32`).

## Secret requirements (greentic-secrets)
- Required secrets: `WEBEX_BOT_TOKEN` (`packs/messaging-webex/pack.manifest.json:57`).

## WASM components (gtpack component sources)
| component | wasm_path | digest |
| --- | --- | --- |
| diagnostics___room_check | components/diagnostics___room_check.wasm | sha256:5ec85da53ba2087a2990ffe996ee27702cab8123eed368f842907d112e643d00 |
| diagnostics___summary | components/diagnostics___summary.wasm | sha256:5ec85da53ba2087a2990ffe996ee27702cab8123eed368f842907d112e643d00 |
| diagnostics___token_check | components/diagnostics___token_check.wasm | sha256:5ec85da53ba2087a2990ffe996ee27702cab8123eed368f842907d112e643d00 |
| diagnostics___webhook_check | components/diagnostics___webhook_check.wasm | sha256:5ec85da53ba2087a2990ffe996ee27702cab8123eed368f842907d112e643d00 |
| requirements___summary | components/requirements___summary.wasm | sha256:5ec85da53ba2087a2990ffe996ee27702cab8123eed368f842907d112e643d00 |
| setup_custom___apply | components/setup_custom___apply.wasm | sha256:5ec85da53ba2087a2990ffe996ee27702cab8123eed368f842907d112e643d00 |
| setup_custom___collect | components/setup_custom___collect.wasm | sha256:5ec85da53ba2087a2990ffe996ee27702cab8123eed368f842907d112e643d00 |
| setup_custom___summary | components/setup_custom___summary.wasm | sha256:5ec85da53ba2087a2990ffe996ee27702cab8123eed368f842907d112e643d00 |
| setup_custom___validate | components/setup_custom___validate.wasm | sha256:5ec85da53ba2087a2990ffe996ee27702cab8123eed368f842907d112e643d00 |
| setup_default___apply | components/setup_default___apply.wasm | sha256:5ec85da53ba2087a2990ffe996ee27702cab8123eed368f842907d112e643d00 |
| setup_default___collect | components/setup_default___collect.wasm | sha256:5ec85da53ba2087a2990ffe996ee27702cab8123eed368f842907d112e643d00 |
| setup_default___summary | components/setup_default___summary.wasm | sha256:5ec85da53ba2087a2990ffe996ee27702cab8123eed368f842907d112e643d00 |
| setup_default___validate | components/setup_default___validate.wasm | sha256:5ec85da53ba2087a2990ffe996ee27702cab8123eed368f842907d112e643d00 |
| verify_webhooks___steps | components/verify_webhooks___steps.wasm | sha256:5ec85da53ba2087a2990ffe996ee27702cab8123eed368f842907d112e643d00 |

Component sources reference: `docs/audit/packs/_evidence/manifests/messaging-webex.manifest.json:528`.

Provider wasm path (no digest in pack manifest or lock):
- `components/messaging-provider-webex.wasm` (`packs/messaging-webex/pack.manifest.json:269`).

## PUBLIC_BASE_URL
- Not found in repo search (`docs/audit/packs/_evidence/rg/public_base_url.txt:1`).

## Subscriptions lifecycle
- No subscriptions extension declared; only provider ops are listed (`packs/messaging-webex/pack.manifest.json:18`).

## Offline testability
- None stated in pack README (`packs/messaging-webex/README.md:1`).

## Status
- PARTIAL: dist manifest version mismatch (0.4.13) vs pack manifest (0.4.15) (`docs/audit/packs/_evidence/manifests/messaging-webex.manifest.json:1223`, `packs/messaging-webex/pack.manifest.json:3`).
