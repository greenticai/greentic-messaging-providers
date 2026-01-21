# MP-PR-02 — Provisioning contract alignment: make setup flows functional (collect/validate/apply/summary) across ALL providers

REPO: greentic-ai/greentic-messaging-providers

GOAL
Make setup/provisioning flows **functional and standardized** for all messaging providers so `greentic-provision` and `greentic-messaging dev setup` can run them reliably.

TARGET STATE
Each provider pack that supports setup exposes:
- entry flow `setup` (in `meta.entry_flows`)
- a deterministic wizard lifecycle:
  - collect -> validate -> apply -> summary
- dry-run mode produces a deterministic `ProvisionPlan` (patches/ops) and never performs network calls
- live mode (optional) is host-gated, not default

NON-GOALS
- Do not move runtime logic into packs that belongs in host services (config/secrets/oauth/subscriptions/http).
- Do not implement cloudflared or runtime routing here.

DELIVERABLES

1) Standardize setup entry + phases
For every provider pack:
- Ensure `setup` exists in `meta.entry_flows` if provider needs setup.
- Ensure setup steps are clearly identified as:
  - `<setup_variant>__collect`
  - `<setup_variant>__validate`
  - `<setup_variant>__apply`
  - `<setup_variant>__summary`
If the pack uses a single flow, it must accept an explicit `step` in input and behave equivalently.

2) Public URL requirement for webhook-based providers
For providers that receive inbound webhooks:
- Slack, WhatsApp, Teams, WebEx, Telegram, (WebChat if webhook-based)
Declare `public_base_url` as a required setup input:
- add to config schema OR a setup-input schema (preferred)
- include it in requirements output
- the apply plan must include webhook endpoint(s) derived from it

3) Apply phase must be functional in dry-run
In dry-run:
- DO NOT call external APIs.
- Emit a deterministic plan with:
  - `config_patch` (merge patch)
  - `secrets_patch` (set/delete; values may be placeholder/redacted)
  - `webhook_ops` (register/update/delete desired endpoints)
  - `subscription_ops` (desired subscription targets/renewal settings)
  - `oauth_ops` (if needed; represent as ops, do not execute)

4) Requirements flow must be consistent
If a provider has a `requirements` entry:
- Ensure it returns machine-readable requirements:
  - required config keys (including public_base_url where relevant)
  - required secret keys
  - oauth required (Y/N)
  - subscriptions required (Y/N)
If it does not exist, add it (even if simple).

5) Documentation (pack-local)
Update pack docs/metadata to include:
- setup inputs (what user/host must provide)
- what will be written (config/secrets)
- webhook endpoints used (if any)
- subscriptions notes (if any)

ACCEPTANCE CRITERIA
- Every provider pack with setup can run dry-run setup deterministically and produce a plan.
- Webhook-based providers declare `public_base_url` requirement.
- No external network calls occur in dry-run.
