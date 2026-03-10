# Provider Pack Audit Matrix (v0.4.34)

All packs migrated to capability-driven pattern. Legacy flows and component stubs removed.

| Pack | Version | Components | Flows | Ingress | Extensions | requires_setup |
|------|---------|:---:|:---:|:---:|------------|:---:|
| messaging-dummy | 0.4.34 | 1 | 2 | No | capabilities, provider-ext | false |
| messaging-email | 0.4.34 | 1 | 2 | No | capabilities, provider-ext | true |
| messaging-webchat | 0.4.34 | 1 | 2 | Inline | capabilities, provider-ext, ingress | true |
| messaging-telegram | 0.4.34 | 2 | 2 | Separate | capabilities, provider-ext, ingress | true |
| messaging-slack | 0.4.34 | 2 | 2 | Separate | capabilities, provider-ext, oauth, ingress | true |
| messaging-teams | 0.4.34 | 2 | 2 | Separate | capabilities, provider-ext, ingress | true |
| messaging-webex | 0.4.34 | 1 | 2 | No | capabilities, provider-ext | true |
| messaging-whatsapp | 0.4.34 | 2 | 2 | Separate | capabilities, provider-ext, ingress | true |

## Extension Keys

- **capabilities** = `greentic.ext.capabilities.v1` (NEW — capability offer with `messaging.configure` op)
- **provider-ext** = `greentic.provider-extension.v1` (provider type, ops, runtime binding)
- **ingress** = `messaging.provider_ingress.v1` (webhook ingress configuration)
- **oauth** = `messaging.oauth.v1` (OAuth 2.0 configuration)

## Removed

- `greentic.messaging.validators.v1` — removed from all packs
- `messaging.provider_flow_hints` — removed from all packs
- Legacy component WASMs (provision, questions, templates, flow-node stubs) — removed
- Legacy flows (diagnostics, setup_custom, verify_webhooks, etc.) — removed
