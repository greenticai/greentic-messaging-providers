# MP-PR-03 — Slack provider pack: functional OAuth setup + ingress/egress fixtures + conformance metadata

REPO: greentic-ai/greentic-messaging-providers

GOAL
Make Slack provider pack fully functional and testable end-to-end (dry-run), covering:
- requirements
- setup (OAuth + webhook endpoints, using public_base_url)
- ingress (handle-webhook)
- egress (send/reply)
- fixtures and expected outputs for conformance

DELIVERABLES

1) Requirements
- Ensure Slack pack requirements declare:
  - config schema path under `schemas/messaging/slack/config.schema.json`
  - secrets needed (client id/secret, signing secret, bot token etc.)
  - oauth required = yes
  - webhook required = yes
- Ensure `assets/secret-requirements.json` lists required secret keys.

2) Setup flow (collect/validate/apply/summary)
- collect:
  - ask for workspace/app identifiers if needed
  - accept `public_base_url` from host (required)
- validate:
  - validate presence and formats of required fields
- apply (dry-run):
  - emit webhook_ops with endpoint paths:
    - events endpoint
    - interactions endpoint (if used)
  - emit oauth_ops describing required redirect URL(s)
  - emit config_patch to store Slack install metadata
  - emit secrets_patch to store tokens placeholders (or mark “to be written by oauth callback”)
- summary:
  - explain next steps (install URL, endpoints)

3) Ingress
- Ensure `handle-webhook` op exists (or the pack’s equivalent)
- Add fixtures:
  - `fixtures/ingress.request.json` containing headers + body (example Slack event)
  - `fixtures/ingress.expected.message.json` describing normalized ChannelMessage invariants
- Validation must cover signature fields (dry-run may skip cryptography but must validate presence and return diagnostic if missing).

4) Egress
- Ensure send op exists and supports dry-run request summary
- Add fixtures:
  - `fixtures/egress.request.json`
  - `fixtures/egress.expected.summary.json` (url/path/body invariants)

5) Pack bundling
- Ensure all schemas/assets/fixtures are bundled in gtpack and appear in SBOM.

6) Live mode (optional, gated)
- Add ignored test or documented steps requiring:
  - RUN_LIVE_TESTS=true, RUN_LIVE_HTTP=true
- Do not run in CI by default.

ACCEPTANCE
- Slack pack passes doctor validation.
- Slack pack passes messaging conformance runner in dry-run when fixtures are used.
