# MP-PR-07 — WebChat (Bot Framework) pack: standardize ingress op + fixtures

REPO: greentic-ai/greentic-messaging-providers

GOAL
Make WebChat pack consistent and testable:
- clarify ingress op naming (handle-webhook vs ingest) and standardize metadata
- setup (if any) declares required inputs
- ingress/egress fixtures

DELIVERABLES
1) Decide and standardize ingress operation:
- If using `handle-webhook`, expose that and update provider declaration ops list.
- If using `ingest`, ensure conformance knows that’s the ingress op.
- Document in pack metadata.

2) Setup
- If Bot Framework requires external endpoint registration, require public_base_url.
- Otherwise, setup writes only local config/secrets via plan.

3) Fixtures
- Ingress fixture from Bot Framework:
  - ingress.request.json (headers/body)
  - ingress.expected.message.json
- Egress fixture (reply/send):
  - egress.request.json
  - egress.expected.summary.json

4) Bundling
- ensure schemas/assets/fixtures are in gtpack and SBOM.

ACCEPTANCE
- doctor passes
- conformance dry-run passes for WebChat pack
