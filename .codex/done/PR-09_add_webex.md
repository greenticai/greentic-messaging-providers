# PR-09: Add Webex provider component (ingress + egress + refresh + formatting)

## Reuse mandate (Webex)
Reuse existing Webex provider logic from greentic-messaging (send + webhook).

## Goal
Complete Webex provider component for pack embedding.

## Tasks
- Create `components/webex/` with WIT imports: http client, secrets-store, telemetry (state-store optional).
- Export provider API:
  - `send_message(room_id: string, text: string) -> result<string, provider-error>`
  - `handle_webhook(headers_json: string, body_json: string) -> result<string, provider-error>`
  - `refresh() -> result<string, provider-error>` (no-op if not needed)
  - `format_message(room_id: string, text: string) -> string`
- Secrets requirements:
  - `WEBEX_BOT_TOKEN` (or whatever the old code uses)
  - webhook verification secret if used
- Update `tools/build_components.sh` to build `target/components/webex.wasm`.
- Unit tests: formatting parity, webhook parse/verify parity, no secret leakage.

## Acceptance
- `target/components/webex.wasm` produced
- ingress + egress + refresh (if needed) + formatting implemented
- secrets-store-only, tests green
