# PR-MP-01: Per-provider migration to universal ops (greentic-messaging-providers)

**Repo:** greentic-messaging-providers  
**Status:** Ready for implementation (explicit steps)  
**Hard rule:** Do NOT change subscription logic (Teams/Graph subscription creation/renewal; Email inbound polling/IMAP/SMTP subscription if any). If you find code paths mentioning `subscription`, `notificationUrl`, `expirationDateTime`, `renew`, `changeType`, `resource`, stop and leave unchanged. This PR only changes webhook ingress normalization and outbound send path surface.

## 0) Outcome we want
Every messaging provider component implements these op strings (component@0.5.0 invoke):
- `ingest_http`
- `render_plan`
- `encode`
- `send_payload`

Legacy ops remain temporarily but are bridged internally:
- `handle-webhook` (legacy) must call `ingest_http` or share the same implementation.
- `send`/`reply` (legacy) may remain, but `send_payload` becomes the new canonical outbound op for operator.

## 1) Where the “old runner-host logic” lives and how to port it
Historically, provider-specific ingress lived partly in runner-host/adapters. In this repo, most of that logic already exists in:
- `components/messaging-ingress-<provider>/` (normalization/validation)
- `components/<provider>/` (combined component with render + send + diagnostics)
- `components/messaging-provider-<provider>/` (provider-core send/reply using http-client host import)
- plus pack specs under `specs/providers/*.yaml`

**Action for Codex:** For each provider, locate any legacy host adapter logic that is NOT in these components by searching the archived runner-host repo (if present locally) using:
```bash
rg -n --hidden --glob '!*target*' "slack/events|slack/interactive|telegram/webhook|teams/activities|webex/webhook|whatsapp/webhook|webchat/activities" ..
rg -n --hidden --glob '!*target*' "dedupe|event_id|signature|signing secret|verify token|challenge" ..
```
Then port only the normalization/verification parts into `ingest_http` implementation (NOT subscription logic). If runner-host repo is not available locally, do not guess; rely on existing ingress components in this repo.

## 2) Canonical message type for normalization output
Normalize inbound webhooks to JSON representing:
- `greentic_types::ChannelMessageEnvelope`
Fields required:
- id, tenant, channel, session_id
Optional: reply_scope, user_id, correlation_id, text, attachments, metadata

## 3) Universal DTOs (must match operator DTO v1)
Implement JSON DTOs identical to operator’s v1 shapes:
- HttpInV1 / HttpOutV1
- RenderPlanInV1 / EncodeInV1
- ProviderPayloadV1 / SendPayloadInV1

Put them in a shared internal module in this repo:
- `crates/provider-common/src/universal_dto.rs` (preferred)
or, if that crate must stay stable:
- `crates/provider-runtime-config` or a new `crates/messaging-universal-dto`.

**Do not** create per-provider DTO variants.

## 4) Per-provider migration recipe (explicit)
For each provider component listed below, make the smallest change: add op dispatch and call existing logic.

### 4.1 Slack
Targets:
- `components/messaging-provider-slack/src/lib.rs`
- `components/messaging-ingress-slack/src/lib.rs`
- maybe `components/slack/src/lib.rs` (if that is the combined component used by packs)

Steps:
1) Add op dispatch:
   - `ingest_http`: accept HttpInV1; use existing ingress normalization that currently expects `headers_json` + `body_json` (see messaging-ingress-slack).
     - Convert HttpInV1 headers/query/body into the old ingress input format if needed.
     - Slack must support both `/events` and `/interactive` via `HttpInV1.path` or `route` hint.
     - Output HttpOutV1 with `events=[ChannelMessageEnvelope]` for real events; for url_verification/challenge respond via `status/body` with empty events.
2) `render_plan`: reuse existing provider-common render planning if present (search for `RenderPlan` usage). If none, implement minimal:
   - If message.text present: summary_text = text
   - If attachments non-empty: add attachment urls
   - Set tier based on capabilities (supports-buttons etc) if available.
3) `encode`: create ProviderPayloadV1 (content-type + body_b64). If provider already builds Slack JSON payload for send/reply, reuse that.
4) `send_payload`: take ProviderPayloadV1 bytes and call Slack API (existing send path). If current code expects structured input instead of payload bytes, wrap it: decode payload bytes into that structure.
5) Keep legacy `send`/`reply` unchanged; optionally have them call encode+send_payload internally.

**Do not change** Slack “inbound_mode: socket_mode/events_api” configuration other than using inbound webhook path.

### 4.2 Telegram
Targets:
- `components/messaging-provider-telegram/src/lib.rs`
- `components/messaging-ingress-telegram/src/lib.rs`
- `components/telegram/src/lib.rs` (if used)

Steps:
1) `ingest_http`: parse HttpInV1.body_b64; reuse existing normalization that currently takes body_json; output ChannelMessageEnvelope.
2) Bound webhook support:
   - If operator passes binding_id/tenant_hint/team_hint, propagate into metadata or tenant ctx resolution if provider uses it.
3) `render_plan`/`encode`: reuse existing render path (inline keyboard etc). If platform cannot support cards, degrade to text. Keep it minimal and deterministic.
4) `send_payload`: use Telegram send API existing logic.

### 4.3 Teams (IMPORTANT: do not touch subscriptions)
Targets:
- `components/messaging-provider-teams/src/lib.rs`
- `components/messaging-ingress-teams/src/lib.rs`
- `components/teams/src/lib.rs` (if used)

Steps:
1) `ingest_http`: accept HttpInV1; reuse existing ingress normalization that currently does `serde_json::from_str(&body_json)` etc.
   - The inbound activity webhook is in payload; do not add subscription registration/renewal.
2) `render_plan`/`encode`: reuse existing payload building to Graph send message.
3) `send_payload`: call Graph send existing code.
4) Ensure any code paths that mention subscriptions remain untouched. If op name list includes `verify_webhooks`, do not modify those flows.

### 4.4 Webex
Targets:
- `components/messaging-provider-webex/src/lib.rs`
- `components/webex/src/lib.rs`

Steps:
1) `ingest_http`: the repo already has `packs/messaging-webex/fixtures/ingress.request.json` and normalization code in `components/webex/src/lib.rs` that parses body_json and normalizes. Reuse that.
2) `render_plan`/`encode`: reuse existing render that builds Webex payload; degrade cards to text.
3) `send_payload`: reuse existing send path.

### 4.5 WhatsApp
Targets:
- `components/messaging-provider-whatsapp/src/lib.rs`
- `components/messaging-ingress-whatsapp/src/lib.rs`
- `components/whatsapp/src/lib.rs`

Steps:
1) `ingest_http`: must support GET challenge (hub.challenge) and POST events.
   - For GET validation, return HttpOutV1 with status/body and empty events.
2) `render_plan`/`encode`/`send_payload`: reuse existing send logic.

### 4.6 WebChat
Targets:
- `components/messaging-provider-webchat/src/lib.rs`
- `components/webchat/src/lib.rs`
- Spec evidence shows `ingest` already exists.

Steps:
1) Implement `ingest_http` as an alias of existing `ingest`:
   - Parse HttpInV1.body_b64 into whatever `ingest` expects (`raw`, etc.)
2) Keep existing `ingest` op for now but migrate packs to use `ingest_http` later.
3) Implement render_plan/encode/send_payload similarly.

### 4.7 Email (IMPORTANT: do not add inbound subscriptions/polling)
Targets:
- `components/messaging-provider-email/src/lib.rs`
- Possibly `packs/messaging-email/*`

Steps:
1) Implement `ingest_http` as:
   - If no inbound is supported, return HttpOutV1 with 404 and empty events.
   - Do not implement IMAP polling or webhook subscriptions.
2) Implement render_plan/encode/send_payload for SMTP send/reply only.

### 4.8 Dummy
Targets:
- `components/messaging-provider-dummy/src/lib.rs`

Steps:
- `ingest_http`: accept anything, return a deterministic ChannelMessageEnvelope event (for tests).
- render_plan/encode/send_payload: minimal no-op send that returns ok.

## 5) Where to find and reuse logic in this repo (explicit pointers)
Use these proven locations:
- Ingress normalization components:
  - `components/messaging-ingress-slack`
  - `components/messaging-ingress-telegram`
  - `components/messaging-ingress-teams`
  - `components/messaging-ingress-whatsapp`
- Combined providers that already do rendering and payload building:
  - `components/slack`, `components/telegram`, `components/teams`, `components/webex`, `components/webchat`, `components/whatsapp`
- Provider-core send/reply components:
  - `components/messaging-provider-*`

Preferred approach:
- Put the `ingest_http` implementation in the provider-core component that operator loads (often `messaging-provider-*`).
- If normalization logic lives in `messaging-ingress-*`, copy/port minimal code (parsing + mapping) into provider-core so operator only needs one component per provider.

## 6) Op-conflict migration policy (simple)
During this PR:
- Keep old op names intact.
- Add new ops in the same component and internally route to the same implementations.
Example dispatch:
- `handle-webhook` → call `ingest_http` adapter layer (convert legacy input to HttpInV1)
- `send`/`reply` → call encode+send_payload or keep as-is; operator will stop calling them after operator PR lands.

## 7) Tests (explicit)
Update/add tests under `crates/provider-tests/tests/`:
- Add a new test file `universal_ops_conformance.rs` that:
  - loads each provider component
  - asserts `invoke("ingest_http", ...)` returns valid HttpOutV1
  - asserts `invoke("render_plan", ...)` returns valid plan JSON
  - asserts `invoke("encode", ...)` returns ProviderPayloadV1 with non-empty body_b64
  - asserts `invoke("send_payload", ...)` returns ok OR retryable err (depending on fixture mocks)

Re-use existing fixtures under `tests/fixtures/*/inbound/*.json`.
For WhatsApp, include GET challenge fixture.

## 8) Explicit "do not touch" list
- Anything named:
  - `verify_webhooks` flows (unless it is purely calling new ops; better: leave unchanged)
  - `subscriptions.*` fixtures or logic in Teams packs
  - OAuth broker flows (unless directly required by send_payload; leave unchanged)

## 9) Deliverables
- All providers implement the 4 universal ops
- Operator can call these ops consistently across providers
- No subscription logic changed