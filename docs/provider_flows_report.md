# Provider Pack Flows (Capability-Driven)

All messaging provider packs have been migrated to the simplified capability-driven pattern.

## Current Flow Structure

Each provider pack now contains exactly **2 flows**:

| Flow | Purpose | Entrypoint |
|------|---------|------------|
| `setup_default` | Configures provider via `messaging.configure` op | `setup` |
| `requirements` | Validates provider configuration | `requirements` |

Both flows are single-node flows that invoke the provider component's `messaging.configure` operation directly.

## Provider Matrix

| Provider | pack_id | provider_type | Components | Ingress |
|----------|---------|---------------|:---:|:---:|
| Dummy | messaging-dummy | messaging.dummy | 1 | No |
| Email | messaging-email | messaging.email.smtp | 1 | No |
| WebChat | messaging-webchat | messaging.webchat | 1 | Inline |
| Telegram | messaging-telegram | messaging.telegram.bot | 2 | Separate |
| Slack | messaging-slack | messaging.slack.api | 2 | Separate |
| Teams | messaging-teams | messaging.teams.bot | 2 | Separate |
| Webex | messaging-webex | messaging.webex.bot | 1 | No |
| WhatsApp | messaging-whatsapp | messaging.whatsapp.cloud | 2 | Separate |

## Removed Legacy Flows

The following legacy flows have been removed from all packs:

- `default` — replaced by capability-driven invocation
- `diagnostics` — now handled by provider component ops
- `setup_custom` — merged into `setup_default` (qa-spec handles modes)
- `remove` — handled by provider component ops
- `update` — handled by provider component ops
- `sync_subscriptions` — handled by provider component ops
- `verify_webhooks` — handled by provider component ops
- `rotate_credentials` — handled by provider component ops (Slack only)
- `setup_qa` — merged into `setup_default` (Telegram only)

## Removed Legacy Components

The following legacy WASM stubs have been removed:

- `provision.wasm` / `questions.wasm` / `templates.wasm`
- All `setup_default___*.wasm`, `setup_custom___*.wasm` variants
- All `diagnostics___*.wasm` variants
- All `verify_webhooks___*.wasm`, `sync_subscriptions___*.wasm` variants
- All `rotate_credentials___*.wasm` variants (Slack)
- `ai.greentic.component-templates` / `ai.greentic.component-provision` / `ai.greentic.component-questions`

## Extensions

Each pack now declares:

| Extension | Purpose | Present in |
|-----------|---------|------------|
| `greentic.ext.capabilities.v1` | Capability offer for messaging | All 8 |
| `greentic.provider-extension.v1` | Provider type, ops, runtime binding | All 8 |
| `messaging.provider_ingress.v1` | Webhook ingress configuration | Telegram, Slack, Teams, WhatsApp, WebChat |
| `messaging.oauth.v1` | OAuth 2.0 configuration | Slack only |

Removed extensions:
- `greentic.messaging.validators.v1` — operator doesn't consume it
- `messaging.provider_flow_hints` — no longer needed (no lifecycle flows)
