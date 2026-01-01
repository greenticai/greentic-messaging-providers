# .codex/PR-12.md — Build & Publish .gtpack Artifacts to GHCR (OCI)

## Goal
Add a deterministic pipeline that builds **.gtpack** artifacts and publishes them to **GHCR (or any OCI registry)** so downstream tools (e.g., `greentic-messaging`, `greentic-distributor-client`) can **pull packs directly by OCI ref**.

This PR turns this repo into a *pack publisher*:
- Build packs from local pack definitions (and/or pinned component refs)
- Publish `.gtpack` as an OCI artifact
- Emit a lockfile with refs + digests

---

## Assumptions / Repo Structure
This repo must contain pack definitions in one of these forms (use what exists; do not invent new formats unnecessarily):

Preferred:
- `packs/<pack_name>/` (pack sources: manifests/flows/assets)
- `packs/<pack_name>/pack.toml` or whatever packc expects

If packs are not yet present, create a minimal placeholder pack named:
- `packs/messaging-provider-bundle/` (or closest naming)
…and document that more packs will be added later.

---

## Deliverables

### 1) `tools/publish_packs_oci.sh`
Add a script that:
- Builds each pack into a `.gtpack`
- Publishes each `.gtpack` to OCI using `oras`
- Produces `packs.lock.json` containing pack name, version, OCI ref, digest, and build timestamp

#### Script requirements
- Must be safe (no secrets echoed)
- Must fail fast (`set -euo pipefail`)
- Must support custom registry/org/repo via env vars
- Must work on GitHub Actions ubuntu runners

#### Env vars
Required:
- `OCI_REGISTRY` (default: `ghcr.io`)
- `OCI_ORG` (default: `${GITHUB_REPOSITORY_OWNER}`)
- `OCI_REPO` (default: `greentic-packs`)  # can be overridden
- `PACK_VERSION` (default: git tag without `v`, e.g. `1.2.3`)
Optional:
- `ORAS_EXPERIMENTAL` if needed by your oras version
- `PACKS_DIR` (default: `packs`)
- `OUT_DIR` (default: `dist/packs`)

#### Script behavior
For each pack directory under `PACKS_DIR/*`:
1. Build:
   - Use `packc` (or the correct greentic pack tool) to produce:
     - `${OUT_DIR}/${pack_name}.gtpack`
2. Compute digest after pushing:
   - `oras push` returns digest; capture it.
3. Write `packs.lock.json` including:
   - `pack_name`
   - `version`
   - `oci_ref` (e.g. `ghcr.io/<org>/<repo>/<pack_name>:<version>`)
   - `digest`
   - `created_at`
   - optionally: git sha

Lockfile format (example shape; keep stable):
```json
{
  "version": "1.2.3",
  "generated_at": "2025-12-16T00:00:00Z",
  "git_sha": "abc123...",
  "registry": "ghcr.io",
  "org": "greentic-ai",
  "repo": "greentic-packs",
  "packs": [
    {
      "name": "messaging-provider-bundle",
      "file": "dist/packs/messaging-provider-bundle.gtpack",
      "oci_ref": "ghcr.io/greentic-ai/greentic-packs/messaging-provider-bundle:1.2.3",
      "digest": "sha256:...."
    }
  ]
}
Notes on OCI media types
Publish .gtpack as an OCI artifact:

Use a clear media type, e.g.:

application/vnd.greentic.gtpack.v1+zip (or +cbor depending on your format)

Attach at least an annotation/label with:

org.opencontainers.image.source

org.opencontainers.image.revision

org.opencontainers.image.version

2) .github/workflows/publish_packs.yml
Add a release workflow that runs on tags:

Trigger:

push tags like v* (e.g., v1.2.3)

Steps:

Checkout

Install Rust toolchain (stable) if building pack tooling is required

Install oras (use official release install approach)

Build/Install pack tool (packc) if not already present

Login to GHCR:

echo "${{ secrets.GITHUB_TOKEN }}" | oras login ghcr.io -u ${{ github.actor }} --password-stdin

Run:

PACK_VERSION=${GITHUB_REF_NAME#v} OCI_REGISTRY=ghcr.io OCI_ORG=${{ github.repository_owner }} OCI_REPO=greentic-packs tools/publish_packs_oci.sh

Upload:

packs.lock.json as workflow artifact

optionally also upload built .gtpack files as artifacts

Security requirements:

Never print tokens

set +x in scripts

Use GITHUB_TOKEN only (no PAT required)

3) README updates
Update README.md with:

What this repo publishes: OCI-hosted .gtpacks

Versioning rule:

tag vX.Y.Z publishes packs with tag X.Y.Z

(optional) latest tag only if you want it (default: do NOT publish latest unless explicitly required)

How to pull a pack:

oras pull ghcr.io/<org>/<repo>/<pack_name>:<version>

or how greentic-messaging / greentic-distributor-client should reference it

Explain packs.lock.json and how consumers use it for pinning digests

Tests / Validation
Add a minimal CI job (non-release) to validate:

script runs in dry-run mode without pushing

pack build succeeds

Implement a DRY_RUN=1 mode in tools/publish_packs_oci.sh:

builds packs

prints the refs it would push

generates lockfile with "digest": "DRY_RUN"

does not call oras push

Acceptance checks:

DRY_RUN=1 tools/publish_packs_oci.sh works locally/CI

Tag workflow publishes on v* tags

packs.lock.json generated and uploaded

Acceptance Criteria
On tag vX.Y.Z, workflow publishes .gtpack artifacts to GHCR under:

ghcr.io/<org>/<repo>/<pack_name>:X.Y.Z

packs.lock.json contains correct OCI refs and digests.

No secrets are printed in logs.

Dry-run mode works and is used in CI (non-tag) to validate builds.