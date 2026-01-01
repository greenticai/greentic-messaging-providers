# PR-10: Add WhatsApp provider component (ingress + egress + refresh + formatting)

## Reuse mandate (WhatsApp)
Reuse existing WhatsApp provider logic from greentic-messaging (send + webhook verification).

## Goal
Complete WhatsApp provider component for pack embedding.

## Tasks
- Create `components/whatsapp/` with WIT imports: http client, secrets-store, telemetry, state-store (optional).
- Export provider API:
  - `send_message(destination_json: string, text: string) -> result<string, provider-error>`
  - `handle_webhook(headers_json: string, body_json: string) -> result<string, provider-error>`
  - `format_message(destination_json: string, text: string) -> string`
  - `refresh() -> result<string, provider-error>` (no-op if not needed)
- Secrets requirements:
  - `WHATSAPP_TOKEN`
  - `WHATSAPP_PHONE_NUMBER_ID`
  - `WHATSAPP_VERIFY_TOKEN` (ingress verification)
- Update `tools/build_components.sh` to build `target/components/whatsapp.wasm`.
- Unit tests: formatting parity, webhook verification parity, no secret leakage.

## Acceptance
- `target/components/whatsapp.wasm` produced
- ingress + egress + formatting (+ refresh if needed) implemented
- secrets-store-only, tests green