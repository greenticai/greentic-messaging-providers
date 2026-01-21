# MP-PR-01 — Pack bundling correctness: include all referenced schemas/assets, enforce via doctor

REPO: greentic-ai/greentic-messaging-providers

GOAL
Unblock publishing and runtime execution by ensuring every provider pack bundles **all referenced schemas and assets** inside the `.gtpack` archive.
After this PR, `greentic-pack doctor --validate` must pass for every `dist/packs/messaging-*.gtpack`.

PROBLEM (CURRENT)
`greentic-pack doctor` fails during publish because provider declarations reference schema files (e.g. `config.schema.json`) that exist on disk but are not included in the `.gtpack`. The archive has no `schemas/` entries.

NON-GOALS
- No runtime code changes in greentic-messaging.
- No provider behavior changes beyond packaging paths.
- No validator rule changes: fix the packs, not the validator.

DELIVERABLES

1) Canonical in-pack paths (mandatory)
Standardize these in-pack locations for every provider pack:

- Config schema:
  `schemas/messaging/<provider>/config.schema.json`
- Optional additional schemas:
  `schemas/messaging/<provider>/*.schema.json`
- Secret requirements:
  `assets/secret-requirements.json`
- Optional docs referenced by provider metadata:
  `docs/<provider>.md` (or repo convention)

Rules:
- Provider declarations MUST reference the canonical in-pack schema path.
- No references to root-level `config.schema.json` or outside-pack paths.
- Any `$ref` dependencies used by schemas must also be bundled in `schemas/messaging/<provider>/`.

2) Pack definitions explicitly include schemas/assets
Update each provider pack definition (`pack.yaml`, `pack.manifest.json`, or build inputs) so that `greentic-pack build` includes:
- all schemas referenced by provider declarations
- all schemas referenced via `$ref` (local references)
- all assets (secret requirements, requirements metadata)
- any docs referenced

If pack.yaml supports `schemas:` and `assets:` fields:
- add them for every pack
Otherwise:
- update the build pipeline (scripts/templates) to copy these files into the pack staging area before build.

3) SBOM completeness
After build, verify:
- each referenced schema/asset appears in the archive and in `sbom.json`
- no dangling references exist

4) CI gate (mandatory)
Add/extend CI workflow (and local script if present) to run:

```bash
for p in dist/packs/messaging-*.gtpack; do
  greentic-pack doctor --validate --pack "$p"
done
```

Fail CI on any validation errors.

5) Regression tests (repo-local)
Add one automated test that:
- builds `messaging-dummy` pack
- opens the resulting `.gtpack`
- asserts `schemas/messaging/dummy/config.schema.json` exists (or the dummy’s canonical schema path)
- asserts `assets/secret-requirements.json` exists when referenced

ACCEPTANCE CRITERIA
- `publish_packs_oci.sh` no longer fails with “provider config schema missing from the pack archive”.
- `greentic-pack doctor --validate` passes for all `dist/packs/messaging-*.gtpack`.
- Provider declarations reference `schemas/...` paths only.
