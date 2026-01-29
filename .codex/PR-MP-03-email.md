PR-MP-03: Email provider inbound via Microsoft Graph (delegated) + subscriptions ops (greentic-messaging-providers)

Repo: greentic-messaging-providers
Provider: messaging-provider-email
Goal: Receive inbound emails via Microsoft Graph change notifications using delegated permissions (per-user consent), driven by operator’s subscriptions service.
Non-goal: Do not change Teams subscription flows; do not introduce a new operator scheduler here.

0) Outcome we want

messaging-provider-email implements:

Universal messaging ops (already planned/needed):

ingest_http

render_plan

encode

send_payload

New subscription ops (so operator can run “subscriptions as a service” generically):

subscription_ensure

subscription_renew

subscription_delete

Inbound email flow:

Operator subscriptions service → subscription_ensure (email provider) creates Graph subscription
Graph calls operator webhook → operator forwards to provider ingest_http
Provider validates handshake + fetches message via Graph → emits ChannelMessageEnvelope events

1) Update shared DTOs (in crates/messaging-universal-dto)

Add versioned subscription DTOs (new structs), designed for delegated auth:

1.1 SubscriptionEnsureInV1

Add fields (minimum):

v: u32 (=1)

provider: String (= "email")

tenant_hint: Option<String>

team_hint: Option<String>

binding_id: Option<String>

resource: String (for delegated: usually "/me/mailFolders('Inbox')/messages")

change_types: Vec<String> (usually ["created"])

notification_url: String

expiration_minutes: Option<u32>

client_state: Option<String>

user: AuthUserRefV1 (NEW, delegated)

user_id: String (stable per-user key in your system, not necessarily Graph id)

token_key: String (lookup key in secrets store, e.g. msgraph:{tenant}:{user}:refresh_token)

metadata: Option<serde_json::Value>

1.2 SubscriptionEnsureOutV1

v: u32

subscription_id: String

expiration_unix_ms: u64

resource: String

change_types: Vec<String>

client_state: Option<String>

user: AuthUserRefV1 (echo back)

metadata: Option<serde_json::Value>

1.3 SubscriptionRenewInV1/OutV1, SubscriptionDeleteInV1/OutV1

Renew includes subscription_id, expiration_minutes (or absolute), user

Delete includes subscription_id, user

Note: Operator should treat these as opaque provider contracts; operator stores subscription_id, expiration, user, etc.

2) Provider: implement delegated auth helper (email provider)

In components/messaging-provider-email/src/ add a small module (or extend existing one):

auth.rs

get_access_token(user: AuthUserRefV1) -> Result<String, node-error>

Uses delegated OAuth:

reads refresh token from secrets store using token_key

exchanges refresh token for access token using MS Graph token endpoint

caches token in-memory with expiry if you already have a cache pattern (optional)

Config/Secrets conventions (no operator hardcoding):

MS_GRAPH_CLIENT_ID, MS_GRAPH_CLIENT_SECRET, MS_GRAPH_TENANT_ID from provider secrets/config (you already prefix MS_GRAPH_ elsewhere)

refresh token stored per-user: key format recommended:

msgraph:{tenant}:{user_id}:refresh_token

3) Provider: implement subscription ops (email provider)

In components/messaging-provider-email/src/lib.rs:

3.1 Add op dispatch + list_ops

Add:

"subscription_ensure" => subscription_ensure(&input_json)

"subscription_renew" => subscription_renew(&input_json)

"subscription_delete" => subscription_delete(&input_json)

3.2 subscription_ensure

Parse SubscriptionEnsureInV1

Acquire access token via auth::get_access_token(user)

Call Graph Create subscription endpoint

Use resource from input (delegated default should be /me/mailFolders('Inbox')/messages)

notificationUrl = input.notification_url

changeType = join(change_types)

clientState = input.client_state (recommended)

expirationDateTime = now + expiration_minutes (Graph has caps; handle provider-side clamping)

Return SubscriptionEnsureOutV1 with subscription_id + expiration

3.3 subscription_renew

Parse SubscriptionRenewInV1

Acquire access token

Call Graph Update subscription (PATCH) to set new expirationDateTime

Return updated expiration

3.4 subscription_delete

Acquire access token

Call Graph Delete subscription

Return ok

Error mapping

429/5xx → node-error.retryable=true with backoff_ms if present

auth/token failures → retryable only if clearly transient; otherwise permanent error

4) Provider: implement inbound ingest_http for Graph notifications

In components/messaging-provider-email/src/lib.rs:

4.1 Validation handshake (required)

If HttpInV1.method == "GET" and query contains validationToken:

Return HttpOutV1 { status: 200, headers: {"Content-Type":["text/plain"]}, body_b64: base64(validationToken), events: [] }

4.2 Notifications (POST)

Decode HttpInV1.body_b64

Parse Graph change notification JSON

(Optional but recommended) validate clientState matches what you issued (if present)

For each notification item:

Extract message id (often in resourceData.id or parse from resource)

Acquire access token for the user associated with this subscription

How to know user?

Use HttpInV1.binding_id as your subscription key

Expect operator to pass binding_id that encodes/subkeys to stored subscription state (operator will)

Also accept tenant_hint/team_hint for lookup

Fetch the message via Graph:

For delegated /me/... subscriptions, you can fetch /me/messages/{id}

Map to ChannelMessageEnvelope:

text: subject + preview (or subject only)

metadata: graph_message_id, internetMessageId, from, receivedDateTime, link, etc.

Return HttpOutV1 { status: 200, events: [..] } plus empty body

Keep render_plan/encode/send_payload for outbound email (Graph sendMail) as-is or minimal. This PR’s core is inbound.

5) Provider outbound (keep minimal but coherent)

encode should produce ProviderPayloadV1 for Graph sendMail call (JSON)

send_payload calls Graph POST /me/sendMail (delegated) using the same AuthUserRefV1 supplied in SendPayloadInV1.tenant.user (or metadata)

If current SendPayloadInV1 doesn’t carry user identity, extend it minimally (or use metadata_json to pass user ref).

6) Tests (provider-tests)

Add to crates/provider-tests/tests/ (or extend existing suite):

email_subscription_ops.rs (or extend universal_ops_conformance.rs later)

Mock token exchange + Graph create/renew/delete endpoints

Assert DTO parse + retryable error mapping

email_ingest_graph_notifications.rs

Test GET validationToken path

Test POST notification → fetch message → emits 1 ChannelMessageEnvelope

7) Explicit “do not touch”

Do not modify Teams subscription logic/flows.

Do not change operator behavior in this PR.

Do not introduce a scheduler in the provider.

8) Deliverables

Email provider supports subscription_ensure/renew/delete

Email provider ingest_http supports Graph validationToken + notifications and emits normalized events

Delegated auth supported via refresh-token lookup from secrets store

Tests passing with mocked Graph endpoints