# PR-07: Add Telegram provider component (ingress + egress + formatting)

## Reuse mandate (Telegram)
Reuse existing Telegram provider logic from greentic-messaging (bot send + webhook updates).

## Goal
Complete Telegram provider component for pack embedding.

## Tasks
- Create `components/telegram/` with WIT imports: http client, secrets-store, telemetry (state-store optional).
- Export provider API:
  - `send_message(chat_id: string, text: string) -> result<string, provider-error>`
  - `handle_webhook(headers_json: string, body_json: string) -> result<string, provider-error>`
  - `format_message(chat_id: string, text: string) -> string`
- Secrets requirements:
  - `TELEGRAM_BOT_TOKEN`
  - optional webhook secret if used
- Update `tools/build_components.sh` to build `target/components/telegram.wasm`.
- Unit tests: payload formatting, webhook parse normalization, no token leakage.

## Acceptance
- `target/components/telegram.wasm` produced
- ingress + egress + formatting implemented
- secrets-store-only, tests green
