# PR: greentic-messaging-providers — Add offline E2E dry-run CI for all packs

Goal
- Add a GitHub Actions workflow that runs greentic-messaging-test against every messaging provider gtpack in --dry-run mode.
- No secret seeding, no provider calls, safe for pull_request.
- Least privilege permissions to avoid CodeQL flags.

Deliverables
1) .github/workflows/e2e-dry-run.yml
   - triggers: pull_request, workflow_dispatch
   - top-level permissions:
     permissions:
       contents: read
   - matrix: derive providers by listing packs/messaging-*.gtpack (or hardcode if simpler)
   - steps:
     - checkout@v4
     - rust toolchain stable
     - rust-cache
     - cargo build -p greentic-messaging-test
     - run:
       greentic-messaging-test <gtpack> --env dev --tenant ci --team ci --dry-run
   - ensure no secrets are referenced and no env vars are printed

2) docs/ci_e2e.md
   - explain offline dry-run purpose and how to run locally

Acceptance
- Runs on PR without secrets
- Uses only contents:read permission
- Each pack completes dry-run
