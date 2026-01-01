# PR-10D.md (greentic-messaging-providers)
# Title: Telegram provider-core pack (real provider) + config schema + minimal send op

## Goal
Implement Telegram as a provider-core pack.

## Scope
- Implement `invoke("send")` using Telegram Bot API (HTTP import via greentic host http capability)
- Optional later: receive/ingest

## Deliverables
1) Schema:
- `schemas/messaging/telegram/config.schema.json`:
  - bot_token (x-secret)
  - default_chat_id (optional)
  - api_base_url (default https://api.telegram.org)
2) Component:
- `components/messaging-provider-telegram/`
- provider_type: `messaging.telegram.bot`
- ops: `send`
3) Pack:
- `packs/messaging-telegram.gtpack/`
- provider extension inline
4) Tests
- Unit tests: JSON shaping
- Integration tests: mocked HTTP (no live Telegram in CI)
  - use a local HTTP mock if your runner supports it; otherwise keep it as compile-only + schema tests and rely on dummy provider for E2E.

## Acceptance criteria
- Pack is self-describing.
- Send op works with HTTP mock.
