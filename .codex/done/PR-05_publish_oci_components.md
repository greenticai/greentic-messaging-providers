# PR-05: Publish provider component artifacts to OCI

## Goal
Add a release workflow and script to publish built .wasm artifacts to GHCR (or configured OCI registry),
so greentic-messaging can fetch and embed them into packs.

## Tasks
1) tools/publish_oci.sh:
- expects env vars for registry/org
- publishes each wasm as an OCI artifact (oras)
- writes components.lock.json including provider name, ref, digest

2) .github/workflows/publish.yml:
- trigger on tag
- build components
- login to GHCR
- run publish_oci.sh
- upload lockfile as artifact

3) README:
- explain versioning + how to fetch artifacts
- note that users install packs; this repo only provides component artifacts

## Acceptance
- publish workflow is correct and safe (no secrets printed)
- lockfile generated
