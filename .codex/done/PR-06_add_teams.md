# PR-06: Add Teams provider component (ingress + egress + refresh + formatting)

## Reuse mandate (Teams)
This PR MUST reuse existing Teams/Microsoft Graph provider logic from the greentic-messaging repo.
Do NOT reimplement sending, formatting, auth/refresh, or webhook/subscription handling from scratch.

Steps:
1) Locate Teams provider code in greentic-messaging:
   - outbound sender (Graph/Teams post)
   - inbound webhook/subscription handler (Graph notifications)
   - auth/token refresh logic (OAuth)
   - message formatting/mapping utilities
2) Move/refactor into:
   - components/teams/
   - shared parts into crates/provider-common and crates/messaging-core
3) Replace env/URI secret reads with greentic:secrets-store@1.0.0 only.
4) Preserve behavior/payloads; add tests proving parity.

## Goal
Deliver a complete Teams provider component artifact embedded by the Teams provider pack.

## Tasks
- Create `components/teams/` (WIT imports: http client, secrets-store, state-store, telemetry).
- Export provider API:
  - `send_message(destination_json: string, text: string) -> result<string, provider-error>`
  - `handle_webhook(headers_json: string, body_json: string) -> result<string, provider-error>`
  - `refresh() -> result<string, provider-error>`
  - `format_message(destination_json: string, text: string) -> string`
- Secrets requirements (structured):
  - `MS_GRAPH_TENANT_ID`
  - `MS_GRAPH_CLIENT_ID`
  - `MS_GRAPH_CLIENT_SECRET` (or certificate/private key if that’s what you use)
  - any webhook verification secret if used
- Use state-store to cache tokens where appropriate.
- Update `tools/build_components.sh` to build `target/components/teams.wasm`.
- Add unit tests for formatting + token refresh behavior + no secret leakage.

## Acceptance
- `target/components/teams.wasm` produced
- egress + ingress + refresh + formatting implemented
- secrets-store-only, no env/URI
- tests green
