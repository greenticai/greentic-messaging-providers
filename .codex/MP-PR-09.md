# MP-PR-09 — Dummy provider pack: gold-standard offline conformance

REPO: greentic-ai/greentic-messaging-providers

GOAL
Make messaging-dummy the canonical pack used by all CI harnesses for offline E2E.
It must be deterministic and minimal.

DELIVERABLES
- Config schema under `schemas/messaging/dummy/config.schema.json`
- Secret requirements asset included (even if minimal) to test plumbing
- Requirements flow returns deterministic required keys
- Setup flow emits deterministic plan patches
- Ingress fixture: deterministic inbound -> ChannelMessage invariants
- Egress fixture: deterministic outbound -> request summary invariants
- Bundling: everything included in gtpack and SBOM

ACCEPTANCE
- doctor passes
- conformance dry-run passes
- used as first pack in CI matrices
