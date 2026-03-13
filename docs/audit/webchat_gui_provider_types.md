# Audit: `webchat` and `webchat-gui` provider types

## Conclusion

The cleanest model is:

- keep one shared Direct Line backend core
- expose two separate provider packs/types:
  - `messaging.webchat`
  - `messaging.webchat-gui`
- make `webchat-gui` a composition of:
  - the same backend behavior as `webchat`
  - additional packaged GUI assets plus public route metadata

This should **not** be modeled as duplicated backend logic.

It also should **not** be modeled as a single pack advertising multiple provider
types unless operator inventory/discovery is changed first.

## What exists today

### In `greentic-messaging-providers`

`webchat` already exists as a first-class provider.

- The runtime component is a single WebAssembly component with internal module
  splits for config, ops, QA, and Direct Line implementation:
  [components/messaging-provider-webchat/src/lib.rs](/projects/ai/greentic-ng/greentic-messaging-providers/components/messaging-provider-webchat/src/lib.rs#L1)
- The Direct Line backend is already factored as a reusable internal module:
  [components/messaging-provider-webchat/src/directline/http.rs](/projects/ai/greentic-ng/greentic-messaging-providers/components/messaging-provider-webchat/src/directline/http.rs#L31)
- Session/conversation state is tenant-aware and isolated in Direct Line state helpers:
  [components/messaging-provider-webchat/src/directline/state.rs](/projects/ai/greentic-ng/greentic-messaging-providers/components/messaging-provider-webchat/src/directline/state.rs#L35)
- Provider config already carries backend-facing fields such as
  `public_base_url`, `mode`, `route`, `tenant_channel_id`, and `base_url`:
  [components/messaging-provider-webchat/src/config.rs](/projects/ai/greentic-ng/greentic-messaging-providers/components/messaging-provider-webchat/src/config.rs#L4)
- The pack exposes a legacy provider runtime binding plus a separate ingress
  extension:
  [packs/messaging-webchat/pack.yaml](/projects/ai/greentic-ng/greentic-messaging-providers/packs/messaging-webchat/pack.yaml#L39)

Important current limitation:

- The provider pack only declares one provider type, `messaging.webchat`:
  [packs/messaging-webchat/pack.yaml](/projects/ai/greentic-ng/greentic-messaging-providers/packs/messaging-webchat/pack.yaml#L66)

### In `greentic-pack`

`greentic-pack` can already bundle arbitrary assets in `.gtpack` archives:

- generic asset bundling under `assets/...`:
  [docs/pack-format.md](/projects/ai/greentic-ng/greentic-pack/docs/pack-format.md#L12)

It also already has a GUI packaging convention:

- GUI manifests are expected at `assets/gui/manifest.json`
- GUI static files are expected under `assets/gui/assets/*`
- the inspector knows how to read GUI routes, workers, fragments, and count GUI assets:
  [crates/greentic-pack/src/bin/common/inspect.rs](/projects/ai/greentic-ng/greentic-pack/crates/greentic-pack/src/bin/common/inspect.rs#L380)

So asset bundling is not the blocker. The missing piece is not pack storage; it
is runtime consumption by operator.

### In `greentic-operator`

The HTTP ingress server currently knows about:

- control-plane routes
- onboard routes
- hard-coded Direct Line routes: `/token`, `/v3/directline/*`, `/directline/*`
- generic provider ingress: `/v1/{domain}/ingress/{provider}/{tenant}/{team?}/{handler?}`

Evidence:

- route dispatch:
  [../greentic-operator/src/demo/http_ingress.rs](/projects/ai/greentic-ng/greentic-operator/src/demo/http_ingress.rs#L198)
- generic ingress parser:
  [../greentic-operator/src/demo/http_ingress.rs](/projects/ai/greentic-ng/greentic-operator/src/demo/http_ingress.rs#L817)

Important current limitations:

- Direct Line is hard-wired to the `messaging-webchat` pack id:
  [../greentic-operator/src/demo/http_ingress.rs](/projects/ai/greentic-ng/greentic-operator/src/demo/http_ingress.rs#L611)
- tenant resolution for Direct Line is query-string based (`?tenant=`), not path based:
  [../greentic-operator/src/demo/http_ingress.rs](/projects/ai/greentic-ng/greentic-operator/src/demo/http_ingress.rs#L620)
- operator setup injects only a generic `public_base_url`; it does not derive
  separate backend and GUI public URLs:
  [../greentic-operator/src/providers.rs](/projects/ai/greentic-ng/greentic-operator/src/providers.rs#L380)

## Main architectural finding

There are really two separate concerns:

1. provider runtime behavior
2. public web hosting behavior

`webchat` is already a provider runtime. `webchat-gui` is not just "another
provider op set"; it is provider runtime behavior plus hosted static frontend
behavior.

That means the clean boundary is:

- shared backend core
- separate packaging/runtime metadata for hosted GUI

## Why a shared backend core is the right base

This repo already points in that direction:

- `webchat` Direct Line logic is isolated in the `directline` module
- the provider shell mainly dispatches ops and QA behavior

That makes it straightforward to extract or reuse a backend core for:

- Direct Line token issuance
- conversation lifecycle
- activity ingestion
- outbound payload/session handling
- auth/origin enforcement

Recommended implementation shape in this repo:

- create a reusable Rust crate or internal module such as `webchat_backend_core`
- keep `messaging-provider-webchat` as the backend-only provider wrapper
- add a second thin wrapper for `messaging-provider-webchat-gui`, or at minimum a
  second pack that reuses the same backend component with a variant-specific
  config/metadata layer

## Why not one pack with two provider types

Operator inventory/discovery is not shaped for that today.

- inventory maps one discovered identifier per pack path into the active catalog:
  [../greentic-operator/src/bundle_lifecycle.rs](/projects/ai/greentic-ng/greentic-operator/src/bundle_lifecycle.rs#L468)
- provider introspection reads only the first provider entry from the provider
  extension:
  [../greentic-operator/src/demo/runner_host.rs](/projects/ai/greentic-ng/greentic-operator/src/demo/runner_host.rs#L4653)

So although `greentic.provider-extension.v1` is an array shape, operator
behavior is effectively single-primary-provider-per-pack.

Audit recommendation:

- use **two packs**
- do not rely on multi-provider-per-pack semantics for PR-00

## What `webchat-gui` needs that does not exist yet

### 1. Public GUI/static route contribution metadata

Today there is metadata for provider ingress (`messaging.provider_ingress.v1`),
but no generic operator contract for:

- public static routes
- GUI route mounting
- derived public URLs for hosted frontend surfaces

`assets/gui/manifest.json` exists as a packaging convention, but operator does
not currently serve it.

### 2. Generic GUI asset serving in operator

No current operator route handling reads `assets/gui/manifest.json` or serves
`assets/gui/assets/*`.

So `webchat-gui` will require operator support to:

- discover GUI routes from pack metadata or GUI manifest
- mount those routes under a provider/tenant-aware prefix
- serve static files from the pack archive or extracted assets dir
- inject runtime config into HTML/JS boot payloads

### 3. Provider-scoped Direct Line URLs

The target design proposes backend URLs like:

- `/v1/messaging/webchat/{tenant}/directline`
- `/v1/messaging/webchat/{tenant}/events`
- `/v1/messaging/webchat/{tenant}/ws`

That is cleaner than the current global `/token` and `/v3/directline/*`
special-case. It also fixes tenant resolution consistency.

Audit recommendation:

- move Direct Line to provider-scoped path routing in operator
- stop relying on global hard-coded webchat routes for hosted mode

## Recommended PR-00 design

### In `greentic-messaging-providers`

Add two provider packs/types:

- `messaging.webchat`
- `messaging.webchat-gui`

Implementation guidance:

- one shared backend core for Direct Line
- `webchat` = backend-only pack
- `webchat-gui` = backend pack plus GUI assets/metadata

Prefer two thin wrappers over duplicated logic.

### In `greentic-pack`

Do not invent a webchat-specific asset format.

Reuse existing generic capabilities:

- bundle GUI assets under `assets/gui/...`
- keep route metadata in `assets/gui/manifest.json` or a dedicated extension

If operator needs stronger typing than the GUI manifest currently provides, add a
small generic extension such as `greentic.public-routes.v1` instead of a
webchat-specific special case.

### In `greentic-operator`

Keep the logic generic:

- mount provider ingress from provider metadata
- mount GUI/static routes from generic GUI/public-route metadata
- derive backend and GUI public URLs from `public_base_url`
- resolve tenant from path consistently for both backend and GUI surfaces

## Bottom line

The best answer to the main question is:

`webchat-gui` should reuse the existing `webchat` backend through a shared backend
core, but it should be packaged as a separate provider pack/type that adds GUI
assets and public route metadata.

The real work is not in Direct Line reuse. That part is already structurally
close. The real gap is in operator support for public GUI route mounting and
provider-scoped URL derivation.
