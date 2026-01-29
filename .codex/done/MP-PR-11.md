# MP-PR-11 — Conformance fixtures & CI: enforce functional packs before publish

REPO: greentic-ai/greentic-messaging-providers

GOAL
Make “functional provider packs” non-negotiable by adding:
- standardized fixtures for every pack
- CI that runs doctor + fixture checks (and optionally greentic-messaging-test e2e in dry-run)

DELIVERABLES

1) Standard fixture contract (mandatory)
For each `messaging-*.gtpack`, ensure `fixtures/` includes:
- `requirements.expected.json`
- `setup.input.json`
- `setup.expected.plan.json`
- `ingress.request.json` (if ingress)
- `ingress.expected.message.json`
- `egress.request.json` (if egress)
- `egress.expected.summary.json`
- `subscriptions.desired.json` and `subscriptions.expected.ops.json` (if subscriptions)

2) Fixture validation tool (repo-local)
Add a small script or Rust tool to:
- verify fixture files exist per pack capability
- verify JSON parses
- verify required fields/invariants exist (structure only)

3) CI gates (mandatory)
After building packs:
- run `greentic-pack doctor --validate` for each pack
- run fixture validation tool for each pack
Optionally (if available in CI):
- run `greentic-messaging-test e2e --packs dist/packs --dry-run`

4) Artifact upload
On CI failure:
- upload built packs and fixture validation report

ACCEPTANCE
- No pack can be published unless it has fixtures and passes doctor.
- This prevents “half functional” provider packs from shipping.
