PR-11 scope checklist
1) Docs

Root README.md:

what this repo is for (build/publish provider components; users install packs elsewhere)

how to build all components locally

required secrets workflow (secrets-store, no env fallbacks)

how artifacts are published + versioning

Per-provider doc stub (optional but helpful):

required secrets keys

exported functions (send/ingress/refresh/format)

“How to consume from greentic-messaging” section:

expected output paths: target/components/<provider>.wasm

lockfile format and fetch method (OCI refs/digests)

2) CI / build matrix

Ensure CI:

builds every component

runs workspace tests

uploads all WASM artifacts

Optional: split jobs per provider to keep CI fast.

3) Publishing + lockfile

Add/finish:

tools/publish_oci.sh

components.lock.json output (provider → OCI ref → digest)

Release workflow on tags:

build all

publish all

upload lockfile

4) Cross-provider consistency checks

Add a small “conformance” test that ensures each provider component:

exports the expected functions (send/handle_webhook/refresh/format_message)

has component.manifest.json with structured secret_requirements

does not reference env vars (basic rg check in CI)

5) Optional: end-to-end smoke harness (in this repo)

Not mandatory, but nice:

a simple runner harness that can instantiate the component with mocked host imports (http/secrets/state)

proves format_message works and send_message constructs the right request without real network