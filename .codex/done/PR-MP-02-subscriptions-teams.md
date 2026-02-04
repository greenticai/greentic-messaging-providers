PR-MP-02: Teams/MSGraph Subscription Ops (provider-driven) (greentic-messaging-providers)

Repo: greentic-messaging-providers
Status: Ready for implementation
Goal: Expose subscription operations as provider ops so operator can run subscriptions as a generic service.
Hard rule: Do not change ingest_http/render_plan/encode/send_payload behavior except where strictly required to accept subscription notifications (e.g., validation handshake).
Keep old subscription flows/service untouched (if any exist here); this PR only adds new ops + DTO wiring.

0) Outcome we want

messaging-provider-teams (or canonical Teams/MSGraph provider component used by packs) implements these additional ops:

subscription_ensure

subscription_renew

subscription_delete

These ops accept versioned JSON DTOs and return versioned JSON DTOs.

1) Universal DTOs (shared, versioned)

Add to crates/messaging-universal-dto/src/lib.rs (same crate you already use in conformance tests):

1.1 SubscriptionEnsureInV1

Fields (minimum):

v: 1

provider: "teams" | "msgraph" (match your provider naming)

tenant_hint: Option<String>

team_hint: Option<String>

binding_id: Option<String>

resource: String (Graph resource string)

change_types: Vec<String> (e.g. ["created","updated","deleted"])

notification_url: String

expiration_target_unix_ms: Option<u64> or expiration_minutes: Option<u32> (pick one; keep it simple)

client_state: Option<String> (recommended)

metadata: Option<serde_json::Value> (opaque extension bag)

1.2 SubscriptionEnsureOutV1

v: 1

subscription_id: String

expiration_unix_ms: u64

resource: String

change_types: Vec<String>

client_state: Option<String>

metadata: Option<serde_json::Value>

1.3 SubscriptionRenewInV1 / OutV1

includes subscription_id + new expiration target, returns updated expiration

1.4 SubscriptionDeleteInV1 / OutV1

includes subscription_id, returns ok

2) Implement ops in the Teams provider component

Target: components/messaging-provider-teams/src/lib.rs (or whichever component operator instantiates)

2.1 Op dispatch

Extend the existing op router to include:

"subscription_ensure" => subscription_ensure(&input_json)

"subscription_renew" => subscription_renew(&input_json)

"subscription_delete" => subscription_delete(&input_json)

Also add them to list_ops if you expose that.

2.2 Graph calls (provider-owned mechanics)

Implement:

subscription_ensure:

Parse SubscriptionEnsureInV1

Call Graph create subscription endpoint

If it fails because “already exists / conflict” (depends on your model), fall back to renewal/update path as appropriate

Return SubscriptionEnsureOutV1

subscription_renew:

Parse renew DTO

Call Graph update subscription (expirationDateTime)

Return new expiration

subscription_delete:

Call Graph delete subscription

2.3 Strict boundary

Do not add scheduling, persistence, retries beyond returning retryable node-errors.

Map Graph transient errors to node-error { retryable: true, backoff_ms: ... }.

3) Tests (provider side)

Add tests under provider-tests (or the provider’s internal tests) that run with mock HTTP:

subscription_ensure returns a subscription id + expiration

subscription_renew updates expiration

subscription_delete returns ok

Ensure error mapping returns retryable errors for 429/5xx.

4) Deliverables

Teams provider supports subscription_* ops with stable DTOs.

No changes to existing subscriptions worker binaries/services in this repo.

No changes to existing ingest/send pipelines beyond any necessary notification handshake correctness.