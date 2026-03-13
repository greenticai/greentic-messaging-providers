PR-MP-01 — Add `messaging.webchat-gui` and remove operator-owned WebChat routing

Title

Add `messaging.webchat-gui`, package `greentic-webchat`, and move all WebChat routing into provider/pack metadata

Status

Ready for implementation.

This file is the active implementation checklist.

Historical files under `.codex/done/PR-MP-01*.md` are not the target for this work.

## Locked decisions

### Overall intent

- Keep `messaging.webchat` as the backend-only Direct Line provider.
- Add `messaging.webchat-gui` as a second provider pack/type that includes hosted GUI assets.
- Reuse one shared backend core.
- Do not duplicate Direct Line logic.
- All WebChat routing must be provider/pack-declared.
- Operator must be able to drop hard-coded WebChat routes.
- This PR assumes generic static-routes support exists or lands first.
- Do not add a fallback operator special case in this PR.

### Provider model

- `messaging.webchat` = backend-only wrapper.
- `messaging.webchat-gui` = backend wrapper + GUI assets + static-routes metadata.
- `messaging.webchat-gui` is a separate provider type and separate pack.
- Backend behavior must still reuse the same shared backend core.
- Acceptable implementation:
  - a second thin runtime wrapper component, or
  - the same runtime component with variant-specific config/metadata.
- Not acceptable:
  - duplicating Direct Line/backend logic.

### Canonical public route shapes

GUI route base:

- `/v1/web/webchat/{tenant}`

Nested GUI assets:

- `/v1/web/webchat/{tenant}/...`

Backend route base:

- `/v1/messaging/webchat/{tenant}/...`

Do not use:

- `/v1/webchat/...`
- pack-id-shaped public paths
- global operator-owned `/token`
- global operator-owned `/v3/directline/*`
- global operator-owned `/directline/*`

### Installation behavior

- `messaging.webchat` remains available as backend-only.
- `messaging.webchat-gui` must be self-contained from the user point of view.
- Installing `messaging.webchat-gui` must provide backend + GUI in one pack/type.
- `messaging.webchat-gui` must work out of the box without requiring a second manual pack install.

### Static routes contract

- Use `greentic.static-routes.v1`.
- Do not use GUI-specific manifest conventions as the runtime contract.
- Static route metadata should reference normal packaged assets under `assets/...`.
- Use:
  - `source_root: assets/webchat-gui`
  - `index_file: "index.html"`
  - `spa_fallback: "index.html"`

### Runtime config injection

Preferred v1 behavior:

- generate a small runtime config payload that the GUI reads at load time
- avoid brittle HTML template substitution

Acceptable forms:

- generated `config.json`
- generated JS bootstrap payload
- small config/bootstrap endpoint

Preferred choice:

- generated runtime config payload over HTML string substitution

### Minimum injected runtime values

Required:

- backend base URL
- tenant

Optional only if already cleanly supported by `greentic-webchat`:

- public GUI base URL
- locale
- theme/skin
- branding
- auth mode
- allowed origins

Do not turn this PR into a larger runtime theming/config framework.

### Tenant resolution

Canonical model:

- path-based tenant resolution only

Canonical GUI route:

- `/v1/web/webchat/{tenant}`

Do not make query-string tenant resolution the primary model.

### Migration stance

- No long-lived migration mode in operator.
- No new operator compatibility layer in this PR.
- Preferred end state is direct move to provider-scoped routes.
- If a temporary bridge becomes unavoidable, keep it short-lived and outside operator if possible.

### `greentic-webchat` build/packaging

- Packaging must consume a deterministic built artifact for `greentic-webchat`.
- Do not rely on undocumented manual copying.
- Acceptable implementation:
  - sibling-repo build step in CI/release pipeline
  - reproducible local build script
  - checked-in build artifact only if that is already repo norm

Preferred choice:

- deterministic packaging step that builds or imports `greentic-webchat` assets during pack assembly

### Versioning

- The pack is the release unit.
- The bundled GUI assets version is whatever ships inside that pack release.
- If easy, record bundled GUI version/commit in metadata.
- Do not block this PR on designing a larger cross-repo versioning system.

### GUI surface scope

For v1:

- static files
- plus the minimal config/bootstrap mechanism needed to inject backend URL

Not in scope:

- a broader web app backend
- unrelated dynamic APIs

### Test level required

Required:

- pack metadata tests
- asset presence/packaging tests
- backend-only pack tests
- backend+GUI pack tests
- config/bootstrap generation test
- operator/runtime route mounting tests proving routing is provider/pack-declared

Nice if cheap, but not required:

- full browser end-to-end tests

### Dependency assumption

This PR assumes:

- `greentic.static-routes.v1` exists
- operator/setup/start support for static routes exists or lands first

Do not add a fallback special case in this PR.

## Goal

Ship two clean WebChat packs:

- `messaging.webchat` — backend-only Direct Line provider
- `messaging.webchat-gui` — backend + packaged `greentic-webchat` GUI

and ensure all WebChat routes are declared by provider/pack metadata so operator no longer hardcodes them.

## Why

This is the application-level migration that consumes the generic static-routes work.

## Acceptance criteria

- `messaging.webchat` remains backend-only
- `messaging.webchat-gui` is a separate provider pack/type
- `messaging.webchat-gui` packages `greentic-webchat`
- GUI is mounted at `/v1/web/webchat/{tenant}`
- backend routes are under `/v1/messaging/webchat/{tenant}/...`
- GUI is preconfigured out of the box to use the matching backend
- backend logic is shared, not duplicated
- operator no longer hardcodes `/token`, `/v3/directline/*`, or `/directline/*`
- all WebChat routing is pack/provider-declared

## Concrete implementation checklist

### Part A — Shared backend core

#### A1. Extract/reuse Direct Line backend core

- Extract the existing WebChat Direct Line backend logic into a reusable shared core/module.
- Keep shared ownership of:
  - token issuance
  - conversation/session handling
  - activity ingress/egress
  - auth/origin enforcement
  - tenant-aware state behavior
- Do not duplicate any existing Direct Line logic into a second provider implementation.

#### A2. Keep `messaging.webchat` thin

- Keep or refine `messaging.webchat` as the thin backend-only wrapper around the shared core.
- Ensure its pack/type remains backend-only with no hosted GUI surface.

### Part B — Add `messaging.webchat-gui`

#### B1. Create the new pack/type

- Add a new provider pack/type for `messaging.webchat-gui`.
- Decide whether this uses:
  - a second thin runtime wrapper component, or
  - the same runtime component with variant-specific config/metadata.
- Whichever option is used, backend behavior must be shared from the same backend core.

#### B2. Package `greentic-webchat`

- Add the built `greentic-webchat` assets into the pack under normal pack assets.
- Package them under:
  - `assets/webchat-gui/...`
- Ensure packaging is deterministic and reproducible.
- Do not require undocumented manual copying.

#### B3. Declare static GUI routes

- Use `greentic.static-routes.v1`.
- Declare GUI route mount rooted at:
  - `/v1/web/webchat/{tenant}`
- Treat the mount as a prefix mount for nested assets:
  - `/v1/web/webchat/{tenant}/...`
- Route metadata should point to:
  - `source_root: assets/webchat-gui`
  - `index_file: "index.html"`
  - `spa_fallback: "index.html"`

#### B4. Declare backend/provider routes

- Update provider metadata so backend routes are pack/provider-declared.
- Canonical backend namespace:
  - `/v1/messaging/webchat/{tenant}/...`
- Replace reliance on operator hard-coded:
  - `/token`
  - `/v3/directline/*`
  - `/directline/*`

#### B5. Keep `webchat-gui` self-contained

- Ensure `messaging.webchat-gui` exposes both backend and GUI surfaces in one installed pack/type.
- It must not require a second manual install of `messaging.webchat`.
- Internally it must still reuse the shared backend core.

### Part C — Out-of-the-box runtime config

#### C1. Inject backend URL into the GUI automatically

- Generate a minimal runtime config/bootstrap payload that the GUI reads at load time.
- Prefer:
  - generated `config.json`, or
  - a minimal bootstrap/config endpoint
- Avoid HTML string substitution unless unavoidable.

#### C2. Inject minimum required values

- Inject:
  - backend base URL
  - tenant
- Inject optional values only if already cleanly supported by `greentic-webchat`.

#### C3. Ensure correct backend pairing

- `messaging.webchat-gui` must automatically point to the matching backend namespace:
  - `/v1/messaging/webchat/{tenant}/...`
- No manual route wiring should be required after install.

### Part D — Build and packaging pipeline

#### D1. Add deterministic asset import/build step

- Wire a deterministic packaging step that builds or imports `greentic-webchat` assets during pack assembly.
- Keep the build/import path documented and reproducible.

#### D2. Record bundled GUI version if easy

- If low-cost, stamp the bundled GUI version or commit in metadata.
- Do not expand scope into a larger cross-repo version-coupling system.

### Part E — Tests

#### E1. Shared backend core tests

- Keep or expand unit tests for the shared Direct Line backend core.
- Ensure reuse does not regress token, conversation, or activity handling.

#### E2. Backend-only pack tests

- Add or update tests for `messaging.webchat` metadata and route declarations.
- Verify it remains backend-only.

#### E3. Backend+GUI pack tests

- Add or update tests for `messaging.webchat-gui` metadata.
- Verify static-routes declarations are present and correct.
- Verify packaged GUI assets are present in the pack.

#### E4. Runtime config/bootstrap tests

- Add a test proving the generated runtime config/bootstrap payload points the GUI to the correct backend URL.

#### E5. Operator/runtime integration tests

- Add integration coverage sufficient to prove WebChat routing is provider/pack-declared.
- Verify operator/runtime can mount:
  - `/v1/web/webchat/{tenant}`
  - `/v1/messaging/webchat/{tenant}/...`
- Verify operator no longer depends on hard-coded WebChat routes once metadata-based routing is used.

## Repo boundaries

### In `greentic-messaging-providers`

- extract/reuse shared Direct Line backend core
- keep `messaging.webchat` backend-only
- add `messaging.webchat-gui`
- package `greentic-webchat` assets under `assets/webchat-gui/...`
- declare static route mount:
  - `/v1/web/webchat/{tenant}`
- declare backend/provider routes under:
  - `/v1/messaging/webchat/{tenant}/...`
- generate/inject minimal runtime config so GUI points to the correct backend automatically

### In `greentic-operator`

- no WebChat special cases should remain after the dependent operator PR lands
- WebChat routes must come from pack/provider declarations only

### In `greentic-pack`

- static route declaration is via `greentic.static-routes.v1`
- `source_root` uses full asset path, for example:
  - `assets/webchat-gui`

## Explicit non-goals

- Do not duplicate backend logic.
- Do not add operator fallback special cases.
- Do not keep global `/token` or `/v3/directline/*` ownership in operator.
- Do not invent a broader theming/branding/runtime-config framework.
- Do not turn this into a larger web backend project.
- Do not require full browser E2E coverage unless already cheap and established.
