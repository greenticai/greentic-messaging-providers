# Greentic Messaging Providers

WASM-based messaging provider components for the Greentic platform. Each provider is a self-contained WebAssembly component (WASI Preview 2) that bridges the Greentic operator to an external messaging service.

## Providers

| Provider | Channel | Adaptive Card Tier | External API | Secret Keys |
|----------|---------|:---:|-----|------------|
| **Slack** | Slack | TierD (plain text) | `chat.postMessage` | `SLACK_BOT_TOKEN` |
| **Teams** | Microsoft Teams | TierA (native AC) | MS Graph API | `MS_GRAPH_CLIENT_SECRET`, `MS_GRAPH_REFRESH_TOKEN` |
| **Telegram** | Telegram | TierD (plain text) | Telegram Bot API | `TELEGRAM_BOT_TOKEN` |
| **Webex** | Cisco Webex | TierB (AC attachment) | Webex REST API | `WEBEX_BOT_TOKEN` |
| **WebChat** | BotFramework WebChat | TierA (native AC) | Direct Line / local in-process state | None |
| **WhatsApp** | WhatsApp Business | TierD (plain text) | WhatsApp Cloud API | `WHATSAPP_TOKEN` |
| **Email** | Email (Graph API) | TierD (HTML) | MS Graph `/me/sendMail` | `MS_GRAPH_CLIENT_ID`, `MS_GRAPH_REFRESH_TOKEN`, `FROM_ADDRESS`, `GRAPH_TENANT_ID` |
| **Dummy** | Test only | N/A | None | None |

## How It Works

### WASM Component Model

Each provider compiles to `wasm32-wasip2` and is packaged into a `.gtpack` archive (ZIP). The operator loads these archives at startup, instantiates each WASM component via Wasmtime, and dispatches operations through the WIT interface.

```
Operator (Rust / Wasmtime)
  │
  ├── loads .gtpack (ZIP containing WASM + flows + metadata)
  ├── instantiates WASM component in sandbox
  ├── links host imports (provider-dependent; e.g. http-client, secrets-store)
  └── calls invoke(op, input_cbor) → output_cbor
        │
        ├── "ingest_http"   → ingress (webhook → normalized message)
        ├── "render_plan"   → plan adaptive card rendering for this channel
        ├── "encode"        → serialize message into provider-specific envelope
        └── "send_payload"  → deliver envelope to external API
```

### WIT Interface

All providers implement `greentic:component@0.6.1`:

```wit
world component-v0-v6-v0 {
    import http-client;       // HTTP calls to external APIs
    import secrets-store;     // Read credentials via greentic-secrets
    export descriptor;        // Component metadata (describe)
    export runtime;           // invoke(op, input_cbor) → output_cbor
    export qa;                // QA lifecycle (qa-spec, apply-answers)
    export component-i18n;    // Localization (i18n-keys, i18n-bundle)
    export schema-core-api;   // JSON-based invoke (operator v0.4.x compat)
}
```

**Import variants by provider:**
- Most providers: `http-client` + `secrets-store`
- WebChat: `secrets-store` (Direct Line state is handled in-process)

### Dual Export

All providers export both interface versions for backward compatibility:

| Interface | Encoding | Used By |
|-----------|----------|---------|
| `greentic:component@0.6.1` (runtime) | CBOR | Component runtime (v0.6+) |
| `greentic:provider-schema-core/schema-core-api@1.0.0` | JSON | Operator v0.4.x ingress |

The schema-core-api `invoke()` delegates to the same handlers as the v0.6 runtime,
including QA operations (`qa-spec`, `apply-answers`, `i18n-keys`) via a JSON↔CBOR bridge.

## Egress Pipeline (Sending Messages)

When the operator sends a message to a channel, it runs three sequential WASM invocations:

```
1. render_plan(message + metadata)
      → determines AC tier for this channel
      → extracts text summary from AC if needed (TierD)
      → passes AC through unchanged (TierA/B)
      → returns render plan with actions/warnings

2. encode(message + render_plan)
      → serializes into ChannelMessageEnvelope
      → provider-specific payload format

3. send_payload(envelope)
      → decodes ChannelMessageEnvelope
      → resolves secrets (bot tokens, API keys)
      → calls external API (Slack, Telegram, Webex, etc.)
      → returns delivery confirmation
```

### Adaptive Card Tiers

The `greentic-messaging-renderer` crate determines how to handle Adaptive Cards based on channel capabilities:

| Tier | Behavior | Providers |
|------|----------|-----------|
| **TierA** | Native AC rendering, pass-through unchanged | Teams, WebChat |
| **TierB** | AC sent as attachment alongside fallback text | Webex |
| **TierD** | AC downsampled to plain text summary | Slack, Telegram, WhatsApp, Email |

## Ingress Pipeline (Receiving Messages)

When an external service sends a webhook:

```
HTTP webhook → operator routes by provider
    → invoke("ingest_http", raw_request)
    → provider parses webhook payload
    → normalizes to HttpOutV1 response { v: 1, status, headers, body }
    → operator dispatches to egress pipeline
```

All providers return `HttpOutV1` with `"v": 1` for operator v0.4.x compatibility.

## Integration Points

### greentic-secrets

Providers read credentials via the `secrets-store` WIT import. At runtime, `resolve_secret(key)` is fulfilled by the operator's configured secrets backend (local file, AWS Secrets Manager, Azure Key Vault, HashiCorp Vault, etc.).

Each provider's `component.manifest.json` declares `secret_requirements`. Pack builds merge these into `pack.manifest.json` inside the `.gtpack`, so `greentic-secrets` knows exactly which keys each provider needs.

**Secret workflow:**
1. Pack metadata declares required secret key names (never values)
2. Operator seeds secrets via `greentic-secrets init --pack <file>.gtpack`
3. At runtime, provider calls `resolve_secret("SLACK_BOT_TOKEN")` → operator reads from backend → returns value to WASM sandbox

### greentic-operator

The operator is the runtime host. Here's the full lifecycle from startup to message delivery:

#### Step 1: Discovery

On startup (`demo start` or `demo send`), the operator scans the demo bundle for `.gtpack` files:

```
demo-bundle/
└── providers/
    └── messaging/
        ├── messaging-slack.gtpack
        ├── messaging-telegram.gtpack
        ├── messaging-webex.gtpack
        └── ...
```

For each `.gtpack` ZIP, it reads `manifest.cbor` (or `pack.manifest.json`) to extract:
- `pack_id` — canonical provider identifier
- `entry_flows` — list of supported operations (e.g. `["ingest_http", "render_plan", "encode", "send_payload"]`)

Results stored in `catalog: HashMap<(Domain, String), ProviderPack>`.

#### Step 2: WASM Loading & Linker Setup

When an operation is dispatched, the operator:

1. Opens `.gtpack` ZIP → extracts `components/messaging-provider-*.wasm`
2. Creates a `PackRuntime` with Wasmtime and links WIT imports:

| WIT Import | Host Implementation |
|------------|-------------------|
| `greentic:secrets/secrets-store@1.0.0` | `SecretsManagerHandle` → dev secrets file / AWS / Azure KV / Vault |
| `greentic:http/http-client@1.1.0` | Outbound HTTP (provider API calls to Slack, Telegram, etc.) |
| `greentic:state/state-store@1.0.0` | Optional host key-value store (only for components that declare it) |
| `wasi:io/*`, `wasi:random/*` | Standard WASI Preview 2 |

3. Resolves provider binding via `pack_runtime.resolve_provider()`
4. Calls `pack_runtime.invoke_provider(&binding, ctx, op_id, payload)` → WASM executes in sandbox

#### Step 3: Egress Dispatch (demo send)

The `demo send` CLI calls three WASM operations sequentially:

```
ChannelMessageEnvelope
    │
    ├─ invoke("render_plan", RenderPlanInV1)  → RenderPlanOutV1
    │       decides AC tier, extracts text if TierD
    │
    ├─ invoke("encode", EncodeInV1)           → EncodeOutV1
    │       consumes render_plan output payload and serializes ProviderPayloadV1
    │
    └─ invoke("send_payload", SendPayloadInV1) → SendPayloadResultV1
            resolves secrets, HTTP POST to external API
```

Each `invoke()` creates a fresh WASM instance with linked imports.

#### Step 4: Ingress Routing (HTTP Server)

When the operator HTTP server is running (`demo start`), webhooks are routed by URL path:

```
POST /messaging/ingress/{provider}/{tenant}/{team?}
    │
    ├─ parse provider, tenant, team from URL
    ├─ build HttpInV1 { method, headers, body_b64, ... }
    ├─ invoke("ingest_http", input) → HttpOutV1 { status, events[] }
    │
    └─ for each event in events:
        spawn std::thread → dispatch_egress()
            ├─ invoke("render_plan", ...)
            ├─ invoke("encode", ...)
            └─ invoke("send_payload", ...)
```

#### Complete Call Chain

```
HTTP webhook or CLI command
    ↓
DemoRunnerHost::invoke_provider_op()
    ↓
PackRuntime::load(gtpack_path, host_config, secrets, state_store, ...)
    ↓
Wasmtime instantiates WASM component
    ↓
pack_runtime.invoke_provider(binding, ctx, op_id, payload_cbor)
    ↓
WASM executes with sandboxed imports (secrets, http, state)
    ↓
FlowOutcome { success, output: JSON }
```

#### Key Operator Source Files

| File | Purpose |
|------|---------|
| `src/demo/runner_host.rs` | `DemoRunnerHost` — pack loading, linker setup, invoke dispatch |
| `src/demo/http_ingress.rs` | HTTP server routing, ingress → egress dispatch |
| `src/domains/mod.rs` | `.gtpack` discovery, manifest reading |
| `src/messaging_universal/egress.rs` | `build_render_plan_input`, `build_encode_input`, `build_send_payload` |
| `src/cli.rs` | `demo send` / `demo start` CLI entry points |

### greentic-qa (Onboarding Questions)

Each provider implements a QA (Question-Answer) contract for onboarding. When a user
sets up a provider, the operator calls the provider to get a list of setup questions,
collects answers, then sends them back for validation and config generation.

#### QA Lifecycle Modes

| Mode | Purpose |
|------|---------|
| **Default** | Returns current configuration state |
| **Setup** | Initial provider configuration (all required questions) |
| **Upgrade** | Reconfigure existing provider (only changed fields) |
| **Remove** | Cleanup and deprovisioning |

#### How QA Questions Are Defined

Questions are **defined inside each provider** in `describe.rs`, not in any external system:

```rust
// components/messaging-provider-slack/src/describe.rs
pub const SETUP_QUESTIONS: &[QaQuestionDef] = &[
    ("public_base_url", "t.qa.public_base_url", true),
    ("bot_token",       "t.qa.bot_token",       false),
    ("api_base_url",    "t.qa.api_base_url",    false),
    ("default_channel", "t.qa.default_channel",  false),
];
```

Each tuple is `(field_key, i18n_text_key, required)`. The `i18n_text_key` maps to a
localized string returned by the `i18n-bundle` export.

A legacy copy of the same questions exists in `packs/*/assets/setup.yaml` for the
operator's interactive setup wizard. Both sources are kept in sync.

#### QA Invoke Flow (Operator → Provider)

The operator **always** calls QA ops through `schema-core-api invoke()` (JSON encoding),
not the WIT-native `qa` interface (CBOR). The `qa_invoke_bridge` module in
`provider-common` bridges the two:

```
Operator calls invoke("qa-spec", {"mode":"setup"})
    ↓
schema-core-api invoke() in provider lib.rs
    ↓
qa_invoke_bridge::dispatch_qa_ops()
    ↓ matches "qa-spec"
bridge_qa_spec() → qa_spec_for_mode() → returns JSON
```

```
Operator calls invoke("apply-answers", {"mode":"setup", "answers":{...}, "current_config":{...}})
    ↓
qa_invoke_bridge::dispatch_qa_ops()
    ↓ matches "apply-answers"
bridge_apply_answers():
    1. Extract answers + current_config from JSON
    2. Insert current_config as "existing_config" in answers
    3. Encode to CBOR
    4. Call provider's apply_answers_impl(mode, cbor)
    5. Decode CBOR result back to JSON
    ↓
Returns {"ok": true, "config": {...}}
```

The provider's `apply_answers_impl()` is a standalone function (not tied to WIT enums)
that is shared by both the WIT `qa` export and the `schema-core-api` JSON path.

#### Who Stores What

| Component | Action |
|-----------|--------|
| **Provider** | Defines questions, validates answers, returns config (pure function, no side effects) |
| **Operator** | Stores config to `config.envelope.cbor`, seeds secrets to secret store |
| **Provider at runtime** | Reads secrets via `secrets_store::get("SLACK_BOT_TOKEN")` (read-only, operator-injected) |

Providers never write to the secret store. The operator injects a secrets manager into
the WASM linker, and the provider reads by hardcoded key name. Scope resolution
(tenant, team, environment) is handled entirely by the operator.

#### Pack Manifest QA Declaration

For the operator to recognize that a provider supports QA, the pack's `manifest.cbor`
(and `pack.manifest.json`) must declare the three QA ops:

```json
{
  "ops": ["ingest_http", "render_plan", "encode", "send_payload", "qa-spec", "apply-answers", "i18n-keys"]
}
```

The operator's `supports_component_qa_contract()` checks for all three ops before
attempting the QA flow. If any are missing, it falls back to the legacy `setup.yaml` path.

### greentic-i18n

Providers export localization keys and Fluent (`.ftl`) bundles for user-facing strings (QA questions, descriptions, error messages). Bundles live in `i18n/` directories per provider.

## Repository Structure

```
greentic-messaging-providers/
├── components/
│   ├── messaging-provider-slack/        # Slack
│   ├── messaging-provider-teams/        # Microsoft Teams
│   ├── messaging-provider-telegram/     # Telegram
│   ├── messaging-provider-webex/        # Cisco Webex
│   ├── messaging-provider-webchat/      # BotFramework WebChat
│   ├── messaging-provider-whatsapp/     # WhatsApp Business
│   ├── messaging-provider-email/        # Email (Graph API + SMTP)
│   ├── messaging-provider-dummy/        # Test / conformance
│   ├── messaging-provision/             # Pack provisioning wizard
│   └── messaging-secrets-probe/         # Secrets diagnostics
├── crates/
│   ├── messaging-core/                  # Shared message types and envelopes
│   ├── provider-common/                 # Shared provider utilities
│   ├── provider-runtime-config/         # Runtime config resolution
│   ├── provider-tests/                  # Shared test harness
│   ├── greentic-messaging-renderer/     # AC renderer (extract, plan, downsample)
│   ├── greentic-messaging-tester/       # E2E test runner
│   ├── greentic-messaging-cardkit/      # Card building toolkit
│   ├── greentic-messaging-packgen/      # Pack generation
│   └── greentic-messaging-planned/      # Render plan types
├── packs/                               # .gtpack build definitions per provider
│   ├── messaging-slack/
│   ├── messaging-teams/
│   ├── messaging-telegram/
│   ├── messaging-webex/
│   ├── messaging-webchat/
│   ├── messaging-whatsapp/
│   ├── messaging-email/
│   ├── messaging-dummy/
│   └── messaging-provider-bundle/       # Combined bundle pack
├── tools/
│   └── build_components.sh              # WASM build script
├── schemas/                             # JSON Schemas for provider config
└── docs/
    └── testing_guide.md
```

### Provider Module Structure

Each provider (except Dummy) is split into focused modules:

```
components/messaging-provider-*/src/
├── lib.rs          # WIT bindings, Component struct, trait impls, apply_answers, tests
├── config.rs       # ProviderConfig structs, defaults, validation, load_config, secrets access
├── describe.rs     # I18N keys/pairs, QA spec, describe payload, JSON schemas
└── ops.rs          # handle_send, handle_reply, ingest_http, render_plan, encode_op, send_payload
```

Teams and Email have additional modules for their Graph API integrations:

```
# Teams
├── graph.rs        # Graph API subscription CRUD, ExistingSubscription, GraphRequestError
└── token.rs        # OAuth token acquisition (refresh_token / client_credentials)

# Email
├── graph.rs        # Graph API helpers (post/patch/delete/get), subscription management
├── ingress.rs      # Webhook handling (validation, Graph notifications, message fetch)
└── auth.rs         # OAuth token acquisition
```

The `provider-common` crate provides shared utilities used by all providers:

```
crates/provider-common/src/
├── lib.rs              # Shared types (ProviderError, RenderTier, ProviderPayload, etc.)
├── helpers.rs          # Utility functions, CBOR-JSON bridge, schema builders, QA/I18N helpers
├── qa_helpers.rs       # Generic ApplyAnswersResult<C>, RemovePlan, result builders
├── qa_invoke_bridge.rs # JSON bridge for QA ops via schema-core-api invoke()
└── test_macros.rs      # standard_provider_tests! macro for shared test generation
```

## Building

### Prerequisites

- Rust 1.90+ with `wasm32-wasip2` target
- `wit-bindgen` 0.53

```bash
rustup target add wasm32-wasip2
```

### Build All Providers

```bash
SKIP_WASM_TOOLS_VALIDATION=1 ./tools/build_components.sh
```

Built WASMs output to `target/components/messaging-provider-*.wasm`.

**Note:** Uses `cargo build` (not `cargo component build`) due to a WIT resolution bug.

### Build Single Provider

```bash
cargo build --manifest-path components/messaging-provider-slack/Cargo.toml \
    --target wasm32-wasip2 --release
```

### Run Tests

```bash
# All tests (unit + integration, 329+ tests)
cargo test --workspace

# Renderer tests only
cargo test -p greentic-messaging-renderer

# Single provider
cargo test -p messaging-provider-slack
```

### Update .gtpack with Rebuilt WASM

```bash
tmpdir=$(mktemp -d)
mkdir -p "${tmpdir}/components"
cp target/components/messaging-provider-slack.wasm "${tmpdir}/components/"
(cd "$tmpdir" && zip -u /path/to/messaging-slack.gtpack components/messaging-provider-slack.wasm)
zipinfo /path/to/messaging-slack.gtpack  # verify
```

## Running (E2E via Operator)

The `greentic-operator` is the runtime host for providers. All E2E testing goes through it.

### Prerequisites

| Tool | Version | Install |
|------|---------|---------|
| Rust toolchain | 1.90+ | `rustup update` |
| `wasm32-wasip2` target | — | `rustup target add wasm32-wasip2` |
| `greentic-operator` | 0.4.23+ | `cargo binstall greentic-operator` |
| `zip` | any | `apt install zip` |

### 1. Build WASMs

```bash
cd greentic-messaging-providers
SKIP_WASM_TOOLS_VALIDATION=1 ./tools/build_components.sh
```

Output: `target/components/messaging-provider-*.wasm` (8 WASMs).

### 2. Update Demo Bundle

Replace WASMs inside the `.gtpack` archives in the demo bundle:

```bash
DEMO_BUNDLE="/path/to/demo-bundle"
WASM_DIR="target/components"

for provider in dummy email slack teams telegram webchat webex whatsapp; do
  gtpack="${DEMO_BUNDLE}/providers/messaging/messaging-${provider}.gtpack"
  wasm="${WASM_DIR}/messaging-provider-${provider}.wasm"
  [ ! -f "$gtpack" ] || [ ! -f "$wasm" ] && continue

  wasm_entry=$(unzip -l "$gtpack" | grep "messaging-provider-${provider}.wasm" | awk '{print $4}')
  [ -z "$wasm_entry" ] && continue

  tmpdir=$(mktemp -d)
  mkdir -p "${tmpdir}/$(dirname "$wasm_entry")"
  cp "$wasm" "${tmpdir}/${wasm_entry}"
  (cd "$tmpdir" && zip -u "$gtpack" "$wasm_entry")
  rm -rf "$tmpdir"
  echo "Updated: $gtpack"
done
```

### 3. Seed Secrets

Secrets are stored in `demo-bundle/.greentic/dev/.dev.secrets.env` (encrypted AES-256-GCM). Seed them with `greentic-secrets apply` using a SeedDoc JSON file:

```bash
cat > /tmp/secrets.json << 'EOF'
{
  "entries": [
    {"uri": "secrets://dev/default/_/messaging-slack/slack_bot_token", "format": "text", "value": {"type": "text", "text": "<token>"}},
    {"uri": "secrets://dev/default/_/messaging-telegram/bot_token", "format": "text", "value": {"type": "text", "text": "<token>"}},
    {"uri": "secrets://dev/default/_/messaging-webex/bot_token", "format": "text", "value": {"type": "text", "text": "<token>"}},
    {"uri": "secrets://dev/default/_/messaging-email/from_address", "format": "text", "value": {"type": "text", "text": "sender@domain.com"}},
    {"uri": "secrets://dev/default/_/messaging-email/graph_tenant_id", "format": "text", "value": {"type": "text", "text": "<tenant-id>"}},
    {"uri": "secrets://dev/default/_/messaging-email/ms_graph_client_id", "format": "text", "value": {"type": "text", "text": "<client-id>"}},
    {"uri": "secrets://dev/default/_/messaging-email/ms_graph_refresh_token", "format": "text", "value": {"type": "text", "text": "<refresh-token>"}},
    {"uri": "secrets://dev/demo/_/messaging-teams/ms_graph_tenant_id", "format": "text", "value": {"type": "text", "text": "<tenant-id>"}},
    {"uri": "secrets://dev/demo/_/messaging-teams/ms_graph_client_id", "format": "text", "value": {"type": "text", "text": "<client-id>"}},
    {"uri": "secrets://dev/demo/_/messaging-teams/ms_graph_refresh_token", "format": "text", "value": {"type": "text", "text": "<refresh-token>"}}
  ]
}
EOF

greentic-secrets apply \
  --file /tmp/secrets.json \
  --store-path demo-bundle/.greentic/dev/.dev.secrets.env
```

**Notes:**
- Teams and Email share the same Azure AD app (public client, no `client_secret`)
- Teams uses tenant `demo`, Email uses tenant `default`
- See `components/messaging-provider-email/README.md` for how to acquire a refresh token

### 4. Send Test Messages (Egress)

`demo send` exercises the full egress pipeline (`render_plan → encode → send_payload`) without starting the HTTP server:

```bash
export GREENTIC_ENV=dev

# Slack
greentic-operator demo send \
  --bundle demo-bundle --provider messaging-slack \
  --to "C0AFWP5C067" --text "Hello from Greentic" \
  --tenant default --env dev

# Telegram
greentic-operator demo send \
  --bundle demo-bundle --provider messaging-telegram \
  --to "7951102355" --text "Hello from Greentic" \
  --tenant default --env dev

# Webex (auto-detect: Y2lz* = roomId, @ = email)
greentic-operator demo send \
  --bundle demo-bundle --provider messaging-webex \
  --to "user@example.com" --text "Hello from Greentic" \
  --tenant default --env dev

# Email (MS Graph sendMail)
greentic-operator demo send \
  --bundle demo-bundle --provider messaging-email \
  --to "recipient@example.com" --text "Hello from Greentic" \
  --tenant default --env dev

# Teams (MS Graph channel message)
greentic-operator demo send \
  --bundle demo-bundle --provider messaging-teams \
  --to "team_id:channel_id" --text "Hello from Greentic" \
  --tenant demo --env dev

# Dummy (no external call, pipeline validation only)
greentic-operator demo send \
  --bundle demo-bundle --provider messaging-dummy \
  --to "test" --text "Pipeline validation" \
  --tenant default --env dev
```

### 5. Send Adaptive Card (Egress)

Create a test card file, then send with `--card`:

```bash
cat > /tmp/test-card.json << 'EOF'
{
  "type": "AdaptiveCard", "version": "1.3",
  "body": [
    {"type": "TextBlock", "text": "Greentic Demo", "weight": "Bolder", "size": "Large"},
    {"type": "TextBlock", "text": "AC test from messaging provider"}
  ],
  "actions": [
    {"type": "Action.OpenUrl", "title": "Visit Greentic", "url": "https://greentic.ai"}
  ]
}
EOF

# Webex — AC renders natively (TierB)
GREENTIC_ENV=dev greentic-operator demo send \
  --bundle demo-bundle --provider messaging-webex \
  --to "user@example.com" --text "AC Demo" \
  --card /tmp/test-card.json --tenant default --env dev

# Slack — AC downsampled to text (TierD)
GREENTIC_ENV=dev greentic-operator demo send \
  --bundle demo-bundle --provider messaging-slack \
  --to "C0AFWP5C067" --text "AC Demo" \
  --card /tmp/test-card.json --tenant default --env dev
```

### 6. Test Ingress (Webhooks)

`demo ingress` simulates an inbound webhook through the provider's `ingest_http` handler:

```bash
# Slack
GREENTIC_ENV=dev greentic-operator demo ingress \
  --bundle demo-bundle --provider messaging-slack \
  --body '{"event":{"type":"message","text":"hello","channel":"C123","user":"U456"}}' \
  --tenant default --env dev

# Telegram
GREENTIC_ENV=dev greentic-operator demo ingress \
  --bundle demo-bundle --provider messaging-telegram \
  --body '{"update_id":1,"message":{"message_id":1,"chat":{"id":123},"text":"hello","from":{"id":456,"first_name":"Test"}}}' \
  --tenant default --env dev

# Webex
GREENTIC_ENV=dev greentic-operator demo ingress \
  --bundle demo-bundle --provider messaging-webex \
  --body '{"resource":"messages","event":"created","data":{"id":"msg123","roomId":"room456","personEmail":"user@example.com"}}' \
  --tenant default --env dev
```

### 7. Start Operator HTTP Server

For full bidirectional testing (webhooks + egress), start the operator HTTP server:

```bash
GREENTIC_ENV=dev greentic-operator demo start \
  --bundle demo-bundle \
  --cloudflared off --nats off --skip-setup --skip-secrets-init \
  --domains messaging
```

This starts Axum on port 8080 with:
- Webhook ingress endpoints for all loaded providers
- Direct Line endpoints for WebChat (`/v3/directline/*`)
- Health check at `/health`

### 8. WebChat (Browser Demo)

WebChat requires the full operator HTTP server + the webchat SPA:

```bash
# Terminal 1: Start operator
GREENTIC_ENV=dev greentic-operator demo start \
  --bundle demo-bundle \
  --cloudflared off --nats off --skip-setup --skip-secrets-init \
  --domains messaging

# Terminal 2: Start webchat SPA
cd greentic-webchat/apps/webchat-spa && npm run dev
```

Open `http://localhost:5176/local-demo/` in browser. Type a message — it goes through:
```
Browser → Direct Line (port 8080) → WASM ingest_http → egress pipeline → bot reply → browser poll
```

## Testing

### Unit Tests

```bash
# All tests (329+ pass)
cargo test --workspace

# Single provider
cargo test -p messaging-provider-slack

# Renderer only
cargo test -p greentic-messaging-renderer
```

### Per-Crate Breakdown

| Crate | Tests | Notes |
|-------|-------|-------|
| `messaging-provider-dummy` | 7 | QA ops + send |
| `messaging-provider-telegram` | 11 | QA ops + send + ingress |
| `messaging-provider-slack` | 8 | QA ops + send |
| `messaging-provider-teams` | 10 | QA ops + send + config |
| `messaging-provider-webex` | 12 | QA ops + send + ingress |
| `messaging-provider-webchat` | 16 | QA ops + send + integration |
| `messaging-provider-whatsapp` | 11 | QA ops + send + ingress |
| `messaging-provider-email` | 10 | QA ops + send + config |
| `greentic-messaging-renderer` | 35 | 12 ac_extract + 14 planner + 5 downsample + 4 noop |
| `provider-common` | 14 | QA bridge + helpers + shared utilities |
| `provider-tests` (WASM) | 11 | 8 instantiation + 3 QA invoke integration |

### What Unit Tests Cover

Each provider has tests for:
- **QA operations** — `qa_spec` and `apply_answers` for all 4 modes (Default, Setup, Upgrade, Remove)
- **QA invoke bridge** — `qa-spec`, `apply-answers`, `i18n-keys` via schema-core-api JSON path (WASM integration tests)
- **Send pipeline** — `render_plan`, `encode`, `send_payload` with mock HTTP
- **Ingress** — `ingest_http` webhook parsing and normalization

### Known Issues

| Issue | Impact | Workaround |
|-------|--------|------------|
| `cargo component build` fails | WIT resolution bug | Use `cargo build` (build script already patched) |
| `greentic-pack build` may fail for packs importing `state-store` | ABI mismatch on some local toolchain combinations | Rebuild affected component WIT/bindings or replace WASM in `.gtpack` as temporary workaround |
| 5 clippy warnings in renderer | `collapsible_if` lint | Pre-existing in `greentic-messaging-renderer`, not from provider code |

### Troubleshooting

**"Secret not found"** — ensure `GREENTIC_ENV=dev` is set. Secrets backend only resolves `dev`/`test`.

**Messages not arriving** — `--to` must be the provider destination (Slack channel ID, Telegram chat ID), not Greentic channel name.

**WASM build errors** — set `SKIP_WASM_TOOLS_VALIDATION=1`. If WIT deps missing, check `wit/<provider>/deps/provider-schema-core/package.wit` exists.

## Pack Structure (Capability-Driven)

Each provider pack follows the simplified capability-driven pattern (matching `packs/telemetry-otlp/`):

```
packs/messaging-{name}/
├── pack.yaml                    # 1-2 components, 2 flows, capability extension
├── components/
│   ├── messaging-provider-{name}.wasm    # Core provider
│   └── messaging-ingress-{name}.wasm     # Ingress (where applicable)
├── flows/
│   ├── setup_default.ygtc               # Single-node setup flow
│   ├── setup_default.ygtc.resolve.json
│   ├── requirements.ygtc                # Single-node requirements flow
│   └── requirements.ygtc.resolve.json
├── schemas/messaging/{name}/            # JSON schemas
├── fixtures/                            # Test fixtures
└── setup.yaml                           # QA wizard definition
```

**Key extension**: `greentic.ext.capabilities.v1` declares a messaging capability offer.
The operator reads this extension and invokes `messaging.configure` on the provider component directly — no legacy flow-node WASMs needed.

Legacy flows (diagnostics, setup_custom, verify_webhooks, rotate_credentials, etc.) and
their generated component WASMs have been removed. All QA operations (qa-spec, apply-answers,
i18n-keys) are handled natively by the provider WASM component.

## Publishing

### OCI (Components)

Tag releases (`v*`) trigger the publish workflow. Components are pushed to `ghcr.io/<owner>/greentic-messaging-providers/<component>:<tag>`.

```bash
# Manual
OCI_REGISTRY=ghcr.io OCI_NAMESPACE=<org> VERSION=<tag> ./tools/publish_oci.sh
```

### OCI (Packs)

Pack releases push `.gtpack` to `ghcr.io/<org>/greentic-packs/<pack>:<version>` with media type `application/vnd.greentic.gtpack.v1+zip`.

```bash
# Dry run
DRY_RUN=1 tools/publish_packs_oci.sh

# Pull
oras pull ghcr.io/<org>/greentic-packs/messaging-telegram:<version>
```

## Secrets Workflow

Runtime secrets are resolved only through `greentic:secrets-store@1.0.0` — provider code never reads environment variables or the filesystem.

1. `component.manifest.json` declares `secret_requirements` per provider
2. Pack build merges them into `pack.manifest.json` inside `.gtpack`
3. Initialize: `greentic-secrets init --pack dist/packs/messaging-telegram.gtpack`
4. Set values: `greentic-secrets set TELEGRAM_BOT_TOKEN=<value>`
5. At runtime: provider calls `resolve_secret("TELEGRAM_BOT_TOKEN")` → operator reads from backend

No secret values are ever baked into `.gtpack` artifacts or logs.

## Conformance

Workspace tests ensure each provider:
- Exposes expected WIT exports (descriptor, runtime, qa, component-i18n, schema-core-api)
- Has `component.manifest.json` with `secret_requirements`
- Does not reference environment variables (all secrets via `greentic:secrets-store`)
- Declares `config_schema.provider_runtime_config` for host injection

## Provider Details

### Slack

- **API**: Slack Web API (`chat.postMessage`)
- **Secrets**: `SLACK_BOT_TOKEN` (required), `SLACK_SIGNING_SECRET` (optional, webhook verification)
- **AC**: TierD — extracts text summary, no card rendering
- **Ingress**: Slack Events API webhook parsing

### Teams

- **API**: Microsoft Graph API (chats/messages, channels/messages)
- **Auth**: OAuth 2.0 Client Credentials or Refresh Token flow
- **Secrets**: `MS_GRAPH_CLIENT_SECRET`, `MS_GRAPH_REFRESH_TOKEN`, `MS_GRAPH_TENANT_ID`, `MS_GRAPH_CLIENT_ID`
- **AC**: TierA — native Adaptive Card attachments
- **Ingress**: MS Graph webhook subscriptions

### Telegram

- **API**: Telegram Bot API (`/sendMessage`)
- **Secrets**: `TELEGRAM_BOT_TOKEN`
- **AC**: TierD — extracts text summary
- **Ingress**: Telegram webhook update parsing

### Webex

- **API**: Webex REST API (`/messages`)
- **Secrets**: `WEBEX_BOT_TOKEN`
- **AC**: TierB — AC sent as attachment alongside text
- **Destination auto-detect**: `Y2lz*` prefix → roomId, `@` → email, else → email
- **Ingress**: Webex webhook + message detail fetch

### WebChat

- **API**: Direct Line (operator-embedded or Microsoft hosted)
- **Imports**: `secrets-store` (state is stored in-process)
- **AC**: TierA — native rendering in BotFramework-WebChat
- **Modes**: `local_queue` (operator in-memory) or `directline` (Microsoft service)

### WhatsApp

- **API**: WhatsApp Cloud API via Facebook Graph (`/{version}/{phone_number_id}/messages`)
- **Secrets**: `WHATSAPP_TOKEN`, `WHATSAPP_PHONE_NUMBER_ID`, `WHATSAPP_VERIFY_TOKEN`
- **AC**: TierD — plain text only
- **Ingress**: WhatsApp webhook verification + message parsing

### Email

- **API**: Microsoft Graph `/me/sendMail` (primary), SMTP (fallback)
- **Auth**: OAuth 2.0 Refresh Token flow (public client, no client_secret)
- **Secrets**: `FROM_ADDRESS`, `GRAPH_TENANT_ID`, `MS_GRAPH_CLIENT_ID`, `MS_GRAPH_REFRESH_TOKEN`
- **AC**: TierD — Adaptive Card downsampled to HTML email body
- **Token flow**: refresh_token → access_token with `Mail.Send` delegated scope
- **Fallback**: client_credentials grant if `MS_GRAPH_CLIENT_SECRET` is seeded (requires `Mail.Send` Application permission)
- **Ingress**: MS Graph webhook notification parsing

### Dummy

- **Purpose**: Test/conformance provider
- **Behavior**: Returns deterministic SHA256 hash: `dummy:{sha256(message)}`
- **Secrets**: None
- **AC**: N/A (minimal implementation)
