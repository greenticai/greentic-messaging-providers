# Offline provider dry-run CI

Workflow: `.github/workflows/e2e-dry-run.yml`
- Triggers on `pull_request` and `workflow_dispatch`.
- Least-privilege permissions (`contents: read`).
- Builds pack artifacts in dry-run mode (`DRY_RUN=1 ./tools/publish_packs_oci.sh`), then runs `greentic-messaging-test` against each provider pack with `--dry-run` so no real provider calls or secrets are needed.
- Environment defaults: `GREENTIC_ENV=dev`, `GREENTIC_TENANT=ci`, `GREENTIC_TEAM=ci`.

Providers covered (matrix):
- messaging-dummy
- messaging-email
- messaging-slack
- messaging-teams
- messaging-telegram
- messaging-webchat
- messaging-webex
- messaging-whatsapp

Local run
```bash
./tools/build_components.sh
./tools/sync_packs.sh
DRY_RUN=1 ./tools/publish_packs_oci.sh
cargo binstall greentic-messaging-test --no-confirm --locked  # if not installed
greentic-messaging-test --fixtures tests/fixtures packs all \
  --packs dist/packs \
  --glob 'messaging-*.gtpack' \
  --env dev --tenant ci --team ci \
  --dry-run
```

Adding a provider
- Add the new pack id to the matrix in `.github/workflows/e2e-dry-run.yml`.
- Ensure `./tools/sync_packs.sh` and `./tools/publish_packs_oci.sh` include the new pack (they scan `packs/`).
- Re-run the workflow locally or via `workflow_dispatch` to validate.
