# MP-PR-06 — WebEx provider pack: webhook setup + ingress/egress fixtures (dry-run functional)

REPO: greentic-ai/greentic-messaging-providers

GOAL
Make WebEx pack functional and testable (dry-run) with:
- setup that plans webhook registration using public_base_url
- ingress normalization fixtures
- egress send fixtures

DELIVERABLES
- Config schema under `schemas/messaging/webex/config.schema.json`
- Secret requirements asset for token/secret fields
- Setup requires public_base_url; apply emits webhook_ops with endpoint URL/path
- Fixtures:
  - ingress.request.json + ingress.expected.message.json
  - egress.request.json + egress.expected.summary.json
- Bundling: schemas/assets/fixtures in gtpack and SBOM

ACCEPTANCE
- doctor passes
- conformance dry-run passes for WebEx pack
