
# PR-08: Add WebChat provider component (ingress + egress + formatting)

## Reuse mandate (WebChat)
Reuse existing webchat provider logic from greentic-messaging (usually formatting + session routing).

## Goal
WebChat provider component for embedded web chat channel (may be mostly formatting/routing).

## Tasks
- Create `components/webchat/` with WIT imports:
  - messaging/session (if used)
  - state-store (if used)
  - telemetry logger
  - secrets-store only if truly needed
- Export provider API:
  - `send_message(session_id: string, text: string) -> result<string, provider-error>`
  - `handle_webhook(headers_json: string, body_json: string) -> result<string, provider-error>` (if webhooks used)
  - `format_message(session_id: string, text: string) -> string`
- Secrets requirements:
  - none by default; only add if existing logic requires shared secret
- Update `tools/build_components.sh` to build `target/components/webchat.wasm`.
- Unit tests: formatting + session payload shape parity.

## Acceptance
- `target/components/webchat.wasm` produced
- provider API implemented (ingress if applicable)
- tests green