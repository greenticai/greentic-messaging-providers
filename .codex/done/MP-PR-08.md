# MP-PR-08 — Email provider pack: make one concrete mode runnable + fixtures

REPO: greentic-ai/greentic-messaging-providers

GOAL
Make Email provider pack functional and testable (dry-run) in at least one concrete mode.
This PR must choose the mode based on current pack implementation:
- polling OR webhook

DELIVERABLES
1) Requirements
- Explicitly declare mode in requirements output:
  - mode: polling|webhook
- Declare required config/secrets accordingly.

2) Setup
- polling mode:
  - configure schedule, mailbox identifiers, credentials requirements
- webhook mode:
  - require public_base_url
  - configure verification keys
- apply emits plan only in dry-run.

3) Ingress fixture
- polling: sample “email fetched” payload fixture
- webhook: sample inbound webhook payload fixture
- expected normalization fixture

4) Egress fixture
- send email fixture and expected request summary

5) Bundling
- schemas/assets/fixtures included in gtpack and SBOM

ACCEPTANCE
- doctor passes
- conformance dry-run passes for Email pack
