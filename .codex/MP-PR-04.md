# MP-PR-04 — WhatsApp provider pack: webhook setup + ingress/egress fixtures (dry-run functional)

REPO: greentic-ai/greentic-messaging-providers

GOAL
Make WhatsApp pack functional and testable (dry-run) for:
- webhook verification/setup (public_base_url + verify token)
- ingress normalization
- egress send

DELIVERABLES

1) Requirements
- Config schema under `schemas/messaging/whatsapp/config.schema.json`
- Secret requirements asset declaring required keys (token, verify token, app secret if applicable)

2) Setup
- Require `public_base_url`
- Collect verify token + any meta app identifiers
- Apply (dry-run) emits webhook_ops:
  - callback URL = public_base_url + provider-specific path
  - verify token setting
- Summary lists endpoint and required provider-side settings

3) Ingress
- Add `fixtures/ingress.request.json` (sample webhook payload + headers)
- Add `fixtures/ingress.expected.message.json` (normalized fields)
- Validate signature/verify (presence at minimum)

4) Egress
- Add `fixtures/egress.request.json` (send payload)
- Add `fixtures/egress.expected.summary.json` (request invariants)

5) Bundling
- Ensure fixtures + schemas + secret requirements are bundled and in SBOM.

ACCEPTANCE
- doctor passes
- conformance dry-run passes for WhatsApp pack
