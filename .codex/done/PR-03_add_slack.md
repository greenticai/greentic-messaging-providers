# PR-03: Add Slack provider component (ingress + egress + refresh + formatting)

## Reuse mandate (Slack)

This PR MUST reuse existing Slack provider logic from the greentic-messaging repo.
Do NOT reimplement Slack message sending, formatting, auth/refresh, or webhook verification from scratch.

Steps:
1) Locate Slack provider code in greentic-messaging:
   - outbound sender (egress)
   - inbound webhook handler / verification (ingress)
   - auth/token handling + refresh (if present)
   - message formatting/mapping utilities
   - error mapping + config parsing
2) Move or refactor that logic into greentic-messaging-providers:
   - components/slack/ for provider-specific component code
   - shared pieces into crates/provider-common and crates/messaging-core
3) Replace env/URI secret reads with greentic:secrets-store@1.0.0 only.
4) Preserve behavior and payload formats; add tests to prove formatting/request-building matches the old implementation.
5) Ensure no secret bytes/tokens appear in logs or error strings.

---

## Goal
Deliver the first complete “one pack per provider” component for Slack:
- egress (send message)
- ingress (handle webhook/event)
- refresh/token lifecycle (if Slack refresh/auth logic exists)
- message formatting/mapping
All in a single Slack component WASM artifact.

This component will later be embedded into the Slack provider pack by greentic-messaging.

---

## Tasks

### 1) Create Slack component crate
Create `components/slack/` with:
- `Cargo.toml`
- `src/lib.rs`
- `wit/` directory with structurally valid WIT packages
- `component.manifest.json` declaring structured secret requirements
- build output `target/components/slack.wasm`

### 2) WIT interface (import host capabilities + export provider API)
#### Imports
Slack component must import these v1 worlds (use greentic-interfaces canonical WIT; do not invent duplicates):
- `greentic:http/client@1.0.0` (outbound HTTP)
- `greentic:secrets-store/store@1.0.0` (secrets)
- `greentic:state/store@1.0.0` (optional but recommended for cursors/token caching)
- `greentic:telemetry/logger@1.0.0` (optional but recommended for safe logs)

#### Exports (provider API)
Export a Slack provider world with at least these functions (names can be adjusted to match existing messaging conventions, but keep stable and documented):

1) **Egress**
- `send_message(channel: string, text: string) -> result<string, provider-error>`
  - Return a stable ack JSON string or message id; keep deterministic.

2) **Ingress**
- `handle_webhook(headers_json: string, body_json: string) -> result<string, provider-error>`
  - Inputs are raw HTTP headers/body encoded as JSON strings to avoid complex WIT types initially.
  - Must verify Slack signature if `SLACK_SIGNING_SECRET` is configured/required.
  - Output is a stable JSON string representing the normalized inbound event (or an ack payload).

3) **Refresh / auth**
- If Slack refresh logic exists in greentic-messaging, expose:
  - `refresh() -> result<string, provider-error>`
  - or `refresh(credentials_json: string) -> result<string, provider-error>`
  - If Slack truly does not support refresh in your current implementation, implement `refresh()` as a no-op returning `{ "ok": true, "refresh": "not-needed" }` but keep the API reserved.

4) **Formatting**
- Provide a pure formatting entrypoint for testability:
  - `format_message(channel: string, text: string) -> string`
  - Should return the exact JSON payload that will be posted to `chat.postMessage`.

### 3) Reuse existing Slack logic from greentic-messaging
- Move/refactor Slack request construction, event parsing, signature verification, and any token/auth handling.
- Extract shared helpers into:
  - `crates/messaging-core` (shared message model, normalization types)
  - `crates/provider-common` (HTTP helpers, OAuth/token helpers, error mapping, redaction)

Do not leave duplicated logic behind without a TODO reference; prefer moving code where possible.

### 4) Secrets migration (no env, no URIs)
All secrets must come from `greentic:secrets-store@1.0.0`.

Declare structured `secret_requirements` in `components/slack/component.manifest.json`:
- Required:
  - `SLACK_BOT_TOKEN` (tenant scope)
- If ingress verification is enabled/implemented:
  - `SLACK_SIGNING_SECRET` (tenant scope)
- Optional (only if your old code used them):
  - app/client credentials for OAuth if applicable

No environment variable fallback is allowed in component code.

### 5) HTTP implementation (safe + consistent)
- Use imported `greentic:http/client@1.0.0` for outbound calls.
- Implement Slack `chat.postMessage` call using the formatted payload.
- Ensure:
  - Authorization header uses token bytes/string from secrets-store
  - no token values are logged
  - error messages redact headers/payloads as needed

### 6) Build integration
Update `tools/build_components.sh` to build Slack component and output:
- `target/components/slack.wasm`

Make build deterministic and fail loudly if the wasm isn’t produced.

### 7) Unit tests (behavior preservation)
Add unit tests that prove we preserved behavior from greentic-messaging:

1) **Formatting parity**
- Given (channel, text), the JSON payload built by the new component matches the payload built by the previous greentic-messaging Slack sender (same fields, same structure).

2) **Ingress verification**
- Verify signature check logic behaves the same as old code (accept valid, reject invalid) using known test vectors (if available in old repo).

3) **No secret leakage**
- Assert that error formatting/redaction does not include the token/signing secret.

Tests should be pure/unit tests where possible (no network).

### 8) Documentation
Update repo README (or component README) briefly:
- Slack component exports: send_message / handle_webhook / refresh / format_message
- Secrets required and how they are provided (secrets-store only)

---

## Acceptance
- `tools/build_components.sh` outputs `target/components/slack.wasm`
- Slack component implements egress + ingress + formatting (+ refresh if applicable)
- `component.manifest.json` declares structured secret_requirements (SLACK_BOT_TOKEN, SLACK_SIGNING_SECRET if used)
- No env/URI secret reads remain in Slack code
- Unit tests pass and demonstrate parity with greentic-messaging Slack behavior
- `cargo test --workspace` is green
