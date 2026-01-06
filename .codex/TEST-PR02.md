# PR: greentic-messaging-providers — Add live E2E (manual + nightly) using seeded file backend secrets

Goal
- Add a workflow that seeds secrets via greentic-secrets init into a file-backed secrets root, then runs greentic-messaging-test live against each provider pack.
- Runs only on workflow_dispatch and schedule.
- No provider logic in greentic-messaging; all tests stay generic by invoking the harness on gtpack inputs.
- Least privilege permissions; avoid CodeQL issues.
- Use GitHub Environment “e2e-live” to store secrets (recommended), but only as env vars injected into the workflow.

Assumptions
- greentic-messaging-test resolves secrets via gsm-core URIs like:
  secrets://<env>/<tenant>/<team>/messaging/<provider>.credentials.json
- greentic-secrets init can seed credentials into a file backend using SECRETS_ROOT/GREENTIC_SECRETS_DIR (confirm the exact env var name used by greentic-secrets + gsm-core; set all relevant ones to the same directory).

Deliverables
1) .github/workflows/e2e-live.yml
   - triggers: workflow_dispatch, schedule (nightly)
   - environment: e2e-live (GitHub Environment)
   - top-level permissions:
     permissions:
       contents: read
   - matrix over packs/messaging-*.gtpack (or explicit list)
   - steps:
     - checkout@v4
     - rust toolchain stable
     - rust-cache
     - build greentic-secrets and greentic-messaging-test (cargo build -p ...)
     - set secrets root (single directory) e.g. ${{ runner.temp }}/greentic-secrets
       export all relevant vars to point gsm-core + greentic-secrets at it:
         - SECRETS_ROOT
         - GREENTIC_SECRETS_DIR
         - (any other used in code: SECRETS_DIR/SECRET_ROOT, etc.)
     - seed secrets:
       greentic-secrets init \
         --pack ci/secrets/<provider>.secrets-pack.yaml \
         --env dev --tenant ci --team ci \
         --non-interactive
     - run live:
       greentic-messaging-test packs/messaging-<provider>.gtpack --env dev --tenant ci --team ci
       If supported, prefer minimal scope:
         --flow smoke  (or the most minimal scenario selector available)
   - do NOT echo secrets, do not enable shell tracing, mask values.

2) Add secrets-pack templates per provider under ci/secrets/
   - telegram.secrets-pack.yaml seeds messaging/telegram.credentials.json
   - email.secrets-pack.yaml seeds messaging/email.credentials.json
   - webex.secrets-pack.yaml seeds messaging/webex.credentials.json
   - Each file contains placeholders referencing env vars injected by GitHub environment secrets.
   - Ensure the resulting seeded JSON matches what provider packs expect.

3) Add docs/ci_live_e2e.md
   - explain required GitHub Environment secrets per provider
   - explain that live workflow is gated and not run on PRs
   - explain file-backed seeding approach and how it maps to secrets:// URIs

Security requirements (CodeQL-friendly)
- permissions:
