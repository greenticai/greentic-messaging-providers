# MP-PR-10 — Bundle pack: include all providers cleanly, consistent paths, no legacy

REPO: greentic-ai/greentic-messaging-providers

GOAL
Ensure the “bundle” pack includes all provider packs/components consistently and does not reintroduce legacy conventions.

DELIVERABLES
- Verify bundle references provider packs using correct ids and schema paths
- Ensure no duplicate provider ids across included providers
- Ensure validator refs (when added) are correct and pinned (optional digest)
- Ensure bundle pack passes doctor and conformance fixture presence checks

ACCEPTANCE
- doctor passes for bundle pack
- conformance dry-run can enumerate included providers without ambiguity
