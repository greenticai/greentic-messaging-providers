# MP-PR-05 — Teams/Graph provider pack: OAuth + subscriptions sync + ingress/egress fixtures (dry-run functional)

REPO: greentic-ai/greentic-messaging-providers

GOAL
Make Teams (Microsoft Graph) pack fully functional and testable (dry-run) for:
- OAuth setup (client id/secret, tenant)
- subscriptions management via sync pattern
- ingress notifications normalization
- egress send message

DELIVERABLES

1) Requirements
- Config schema under `schemas/messaging/teams/config.schema.json`
- Secret requirements include:
  - client secret
  - refresh token storage key(s)
  - signing keys/validation keys if used
- Declare: oauth required yes; subscriptions required yes.

2) Setup flow
- Require `public_base_url` for notifications endpoint if applicable.
- Apply (dry-run):
  - emits oauth_ops (authorization URL + redirect URL derived from public_base_url)
  - emits subscription_ops desired targets and renewal policy
  - emits config_patch for tenant/app ids

3) Subscriptions (sync pattern)
- Standardize as:
  - `sync-subscriptions` op/flow (idempotent reconcile)
- Add fixtures:
  - `fixtures/subscriptions.desired.json`
  - `fixtures/subscriptions.expected.ops.json`
- Ensure sync emits stable ops even when “already present” (no flapping)

4) Ingress
- Fixtures:
  - `fixtures/ingress.request.json` (Graph notification payload)
  - `fixtures/ingress.expected.message.json`
- Ensure normalization includes correlation/thread identifiers where possible.

5) Egress
- Fixtures:
  - `fixtures/egress.request.json`
  - `fixtures/egress.expected.summary.json`

6) Bundling
- Ensure all above is in the gtpack + SBOM.

ACCEPTANCE
- doctor passes
- conformance dry-run passes including subscriptions stage
