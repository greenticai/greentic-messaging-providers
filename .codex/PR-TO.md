Codex Prompt — greentic-messaging-providers

Goal: migrate all provider send operations to accept ChannelMessageEnvelope with to[] + from, starting with Webex as reference.

Global intent (do not debate)

We are standardising all provider send inputs on:

ChannelMessageEnvelope


instead of ad-hoc JSON shapes.

This is not a design discussion. Implement exactly as described.

PR-01: Introduce envelope-based send (Webex only)
Scope

Modify Webex provider only

No other providers touched

Keep behaviour backward-compatible where explicitly stated

Explicit rules (assumed, do not ask)

send input is always JSON-encoded ChannelMessageEnvelope

Destinations are resolved in this order:

envelope.to[0]

provider config default (if any)

otherwise → error

broadcast is out of scope

Attachments are not supported

Text must come from envelope.text

Provider config loading remains unchanged for now

Step 1 — Update send input parsing

Replace any serde_json::Value parsing in Webex handle_send.

Required behavior

Deserialize ChannelMessageEnvelope

Reject invalid JSON

Reject missing or empty text

Reject unsupported attachments

use greentic_types::{ChannelMessageEnvelope, Destination};

let envelope: ChannelMessageEnvelope =
    serde_json::from_slice(input_json)
        .map_err(|e| error("invalid envelope", e))?;

Step 2 — Destination resolution (mandatory)

Implement this logic verbatim:

let destination = match envelope.to.first() {
    Some(dest) => dest.clone(),
    None => {
        if let Some(default_room) = cfg.default_room_id {
            Destination {
                kind: Some("room".into()),
                id: default_room,
            }
        } else {
            return error("destination required");
        }
    }
};

Step 3 — Webex kind mapping

Accepted kinds:

"room" → roomId

"user" → personId

Any other kind → error.

No inference, no fallback.

Step 4 — HTTP send path

Reuse existing HTTP logic

Only change how roomId/personId is selected

Do not change auth, headers, or payload shape beyond destination + text

Step 5 — Tests (Webex)

Update Webex tests to:

Fail when:

to empty AND no config default

Succeed when:

to = [{ kind: "room", id: "…" }]

Mock HTTP is sufficient.

PR-02: Ingress — populate from consistently (Webex)
Scope

Webex ingress only

No send logic changes

Rules

Stop using user_id

Populate envelope.from when sender info exists

from: Some(Actor {
    id: person_id,
    kind: Some("user".into()),
})


to must always be vec![] for ingress

Never synthesize destinations for inbound events

PR-03: Mechanical rollout to other providers
Scope

Telegram

Slack

Teams

WhatsApp

Email

Instructions (do not reinterpret)

For each provider:

Replace send input parsing with ChannelMessageEnvelope

Read text from envelope.text

Resolve destination from envelope.to[0]

Apply provider-specific mapping:

Provider	Accepted destination kinds
Telegram	chat
Slack	channel (optional user → DM if supported)
Teams	channel (teamId:channelId) OR chat
WhatsApp	phone
Email	email

Populate from on ingress

Remove all user_id usage

Ensure all constructed envelopes explicitly set to: vec![]

No new abstractions. Copy Webex pattern.

PR-04: Operator smoke validation (minimal)
Scope

No provider changes

Only ensure operator can send using new shape

Validation

Run:

cargo run --bin greentic-operator -- demo send \
  --provider webex \
  --to room:<id> \
  --text "hello"


Expect:

Provider receives valid ChannelMessageEnvelope

Webex send succeeds or reaches mocked HTTP

Acceptance checklist (non-negotiable)

 Webex send uses ChannelMessageEnvelope

 No provider parses ad-hoc to JSON

 Ingress sets from, not user_id

 All providers compile against updated greentic-types

 Operator demo send works for at least one provider

Codex execution hint (important)

Do not ask clarifying questions.
If a detail is missing, copy the Webex implementation exactly and proceed.

# Codex Task: greentic-messaging-providers — Switch send/ingress to ChannelMessageEnvelope with to[] + to_kind + from (Webex first) + update greentic-messaging-tester

Implement a mechanical refactor. Do NOT ask clarifying questions; make minimal assumptions and proceed. Keep PRs small and sequential. Do providers one-by-one.

## Overall goal
1) Provider **send** operations accept **only** JSON-encoded `greentic_types::ChannelMessageEnvelope` as input.
2) Destinations are resolved from **envelope.to[0] + envelope.to_kind** (first), then provider config default, else error.
3) Stop parsing any ad-hoc `"to": {kind,id}` objects or any provider-specific JSON send shapes.
4) Provider **ingress** (ingest_http) populates `envelope.from` (actor) where possible and never uses `user_id`.
5) Webex is the reference implementation; then roll out the same pattern provider-by-provider (Telegram, Slack, Teams, WhatsApp, Email).
6) Update `greentic-messaging-tester send` to build the envelope using `--to` and `--to-kind`.

## Global conventions
### Destination resolution algorithm (egress send)
For ALL providers:
- If `envelope.to` is non-empty: use the first destination `id = envelope.to[0]`.
- Else if provider config has a default destination: use it and synthesize `id` from config.
- Else: return error `"destination required"`.

`kind = envelope.to_kind` is optional and provider-defined default applies if missing.

### Ingress conventions
For ALL providers:
- Always set `envelope.to = vec![]` for inbound events.
- Populate `envelope.from` when sender info exists:
  - `from: Some(Actor { id: <sender_id>, kind: Some("user".into()) })`
- Do NOT use or populate `user_id` anywhere.

### Attachment convention (for now)
For ALL providers (unless already properly supported):
- Reject attachments on send: error `"attachments not supported"`.

---

## PR plan (execute in order, one PR at a time)

# PR-01 (Webex send): Envelope input + destination mapping with exact Webex keys
Scope: Webex provider only.

## A. Update the Webex send entrypoint(s)
- Identify the exported/entry function(s) used for egress send (e.g. `handle_send`, `send_payload`, `format_message`, etc.).
- Update ONLY send path parsing + destination mapping; reuse existing HTTP logic/auth headers unchanged.

## B. Input shape
- Parse input bytes as `ChannelMessageEnvelope` via serde_json.
- If invalid JSON or cannot deserialize: return `{ ok: false, error: "invalid envelope: ..." }`.
- Reject attachments: if `!envelope.attachments.is_empty()` -> error `"attachments not supported"`.
- Text required: read from `envelope.text`; if missing/empty/whitespace -> error `"text required"`.

## C. Destination resolution (Webex)
Use `envelope.to` + `envelope.to_kind`:
1) If `envelope.to` non-empty: `id = envelope.to[0]`.
2) Else fallback to provider config default destination which MUST be a default email (`toPersonEmail`).
   - Use existing config field if present; if none exists, add exactly one new config field for default email.
3) Else: error `"destination required"`.

Webex kind default:
- If `envelope.to_kind` is None => treat as `"email"`.

Reject empty/whitespace id.

## D. Webex payload mapping — MUST use literal JSON fields:
Set EXACTLY ONE of:
- `toPersonEmail`
- `toPersonId`
- `roomId`

Mapping:
- kind missing or `"email"` => `toPersonEmail = id`
- kind `"person"` or `"user"` => `toPersonId = id`
- kind `"room"` => `roomId = id`
- otherwise error `"unsupported destination kind: <kind>"`

Do NOT use other field names. It must literally be `toPersonEmail`, `toPersonId`, `roomId`.

## E. Config source during transition
- Keep existing provider config loading mechanism as-is.
- Do NOT accept new request-embedded config JSON shapes; adapt tests to existing config mechanism.

## F. Tests (Webex)
Update/add tests to cover:
1) `to=[]` and no config default => destination-required error
2) `to=["a@b.com"]` + no `to_kind` => outgoing payload uses `toPersonEmail`
3) `to_kind="email"` => uses `toPersonEmail`
4) `to_kind="person"` => uses `toPersonId`
5) `to_kind="room"` => uses `roomId`
6) unknown `to_kind` => unsupported-kind error
7) missing/empty text => text-required error
8) attachments non-empty => attachments-not-supported error

If HTTP is mocked, assert outgoing JSON contains the exact key names.

Run: `cargo fmt` and provider tests.

---

# PR-02 (Webex ingress): Populate `from`, remove `user_id`, ensure `to=[]`
Scope: Webex provider only.

## A. In ingress (ingest_http)
Where Webex constructs a `ChannelMessageEnvelope` from inbound webhook events:
- Remove any `user_id` field usage; do NOT populate `user_id`.
- Populate `envelope.from` when sender info exists:
  - `from: Some(Actor { id: <person_id_or_sender_id>, kind: Some("user".into()) })`
- Always set `envelope.to = vec![]` for inbound events. Do NOT infer destinations for ingress.
- Keep other normalization logic intact.

Update ingress tests accordingly.

---

# PR-03 (greentic-messaging-tester): Update send CLI to emit ChannelMessageEnvelope with --to and --to-kind
Scope: greentic-messaging-tester only.

## A. CLI flags
Update `send` command to support:
- `--to <string>` (optional; can be repeatable if easy, but only the first is used)
- `--to-kind <string>` (optional)

Do NOT remove existing flags unless clearly obsolete; map them internally if needed.

## B. Envelope construction
The tester MUST construct a JSON `ChannelMessageEnvelope`:
- `text` from `--text` (required; error if missing/empty)
- `to`:
  - `vec![to]` if `--to` provided
  - `vec![]` otherwise
- `to_kind`:
  - `Some(value)` if `--to-kind` provided
  - `None` otherwise
- `from`: `None` (tester does not synthesize actors)
- `attachments`: empty vec
- Any other required envelope fields: populate minimally/defaults; do not invent semantics.

Serialize this envelope to JSON and pass unchanged to the provider send operation.

## C. Backward compatibility
If tester previously had provider-specific destination flags (e.g. `--room-id`, `--chat-id`):
- Keep them temporarily by mapping them to `--to` + `--to-kind` internally OR mark deprecated with a warning.
- Do NOT emit provider-specific JSON shapes anymore.

## D. Tests
Add/update tester tests asserting:
- `send --text hello --to a@b.com` => envelope `to=["a@b.com"]`, `to_kind=null`
- `send --text hello --to ROOM --to-kind room` => envelope `to=["ROOM"]`, `to_kind="room"`

## E. Manual verification (after PR-01)
Confirm:
```bash
greentic-messaging-tester send --provider webex --text "hello" --to someone@example.com
results in a Webex API payload containing toPersonEmail.

PR-04+ (Provider rollout one-by-one): Telegram, Slack, Teams, WhatsApp, Email
Do NOT do all providers in one PR. Do one provider per PR, in this order:

Telegram

Slack

Teams

WhatsApp

Email

For EACH provider PR:

Send changes
Deserialize ChannelMessageEnvelope from JSON input.

Require envelope.text non-empty; reject attachments (unless already supported cleanly).

Resolve destination:

If envelope.to non-empty: use to[0]

Else fallback to provider config default destination (one field)

Else error "destination required"

Determine kind:

Use envelope.to_kind if present

Else use provider default kind (below)

Remove all ad-hoc parsing of {kind,id} objects; only use to[] and to_kind.

Provider kind defaults / accepted kinds:

Telegram: default "chat"

Slack: default "channel"; allow "user" only if DM resolution already exists, else reject "user"

Teams: accept "channel" (id = teamId:channelId) or "chat" (id = chatId); choose default only if already has a config default, else require explicit

WhatsApp: default "phone"

Email: default "email"

Ingress changes
Populate envelope.from instead of any user_id.

Always set envelope.to = vec![] for inbound events.

Tests (minimum)
destination-required error when to empty and no default

success/mock path when to[0] present and to_kind mapped correctly

Run fmt/tests per PR.

PR-Last (Smoke): Operator demo send validation
After Webex is updated, verify operator demo send can drive at least Webex using the new envelope shape (minimal wiring only if required).

Always run:

cargo fmt

relevant cargo test for affected crates

cargo clippy if CI requires it

What’s incorrect / missing in your updated plan
1) Webex default destination + mapping are wrong

You wrote:

fallback to config.default_room_id

map kind to roomId/personId

But your latest requirement is:

default must be toPersonEmail

other options are roomId and toPersonId

and the Webex payload keys must literally be: toPersonEmail, toPersonId, roomId

So PR-01 should say:

fallback to config default email (whatever the existing field is; if you add one, name it explicitly like default_to_person_email)

map kind → payload field:

email (or missing kind) → toPersonEmail

person/user → toPersonId

room → roomId

2) Envelope shape: you must use to[] + to_kind, not dest.id

Your plan currently describes “resolve destinations via envelope.to[0] … map kind values…”, which is good, but it still talks like Destination { kind, id } exists.

Make it explicit everywhere:

id = envelope.to[0] (a string)

kind = envelope.to_kind (optional string; Webex default "email")

3) PR-03 rollout should be “one provider per PR”

You bundled “Mechanical rollout to other providers (PR-03)” into one PR. Earlier you wanted provider-by-provider so Codex doesn’t ask questions and diffs stay small.

So: PR-03 Telegram, PR-04 Slack, PR-05 Teams, PR-06 WhatsApp, PR-07 Email (or similar).

4) You’re missing the greentic-messaging-tester CLI update

You asked earlier about --to and --to-kind — that needs to be in the plan as its own PR (ideally right after Webex send so you can validate quickly).

5) Operator smoke command likely needs --to + --to-kind

You wrote:
--to room:<id>

That only works if the operator already parses that composite format. With the new scheme you described, the reliable smoke test shape is:

--to <id> plus --to-kind room|email|person

(If operator currently expects room:<id>, keep it, but then it must be translated into envelope to[0]=<id> and to_kind="room" before invoking the provider.)

Corrected plan (drop-in replacement)

PR-01: Webex send refactor

Replace ad-hoc JSON parsing with ChannelMessageEnvelope deserialization.

Enforce:

text required (envelope.text)

attachments rejected

Resolve destination:

if envelope.to non-empty → id=envelope.to[0]

else fallback to config default email (not room)

else error "destination required"

Determine kind:

kind = envelope.to_kind.unwrap_or("email") (Webex default)

Map to Webex payload using literal keys:

email/missing → toPersonEmail: id

person/user → toPersonId: id

room → roomId: id

Keep existing HTTP plumbing unchanged.

Update tests:

missing destination + no default → error

success for to_kind=room → payload has roomId

success for default/missing kind → payload has toPersonEmail

PR-02: Webex ingress cleanup

Remove user_id usage entirely.

Always emit to = vec![].

Populate from = Some(Actor { id: person_id, kind: Some("user".into()) }) when sender info exists.

Keep other normalization unchanged.

PR-03: greentic-messaging-tester send CLI

Add --to <string> and --to-kind <string> to send.

Construct ChannelMessageEnvelope:

text from --text

to = vec![--to] (or empty)

to_kind = Some(--to-kind) (or None)

from = None, attachments = []

Deprecate or internally map any old provider-specific destination flags.

Add tests asserting envelope JSON has to and to_kind.

PR-04..PR-08: Rollout provider-by-provider

Do one provider per PR: Telegram → Slack → Teams → WhatsApp → Email.

For each:

send: parse envelope, require text, reject attachments, resolve to[0] + to_kind with provider default kind, map to provider-native destination field(s), reuse existing invoke/HTTP logic

ingress: populate from, set to=[], remove user_id

minimal regression tests

PR-Last: Operator validation

Confirm operator/demo send produces the envelope correctly and provider receives it.

Use:

--to <id> and --to-kind room|email|person (or ensure operator translates room:<id> into those fields)

Run fmt/clippy/tests.

Verdict

✅ Your plan is conceptually right, but as written it’s not valid because it reintroduces:

Webex default room id (should be default email)

wrong payload fields (personId vs toPersonId, etc.)

rollout in one PR (you wanted provider-by-provider)

missing tester CLI update