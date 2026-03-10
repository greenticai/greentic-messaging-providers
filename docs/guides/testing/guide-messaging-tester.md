# Messaging Tester: Validate Providers via WASM Harness

`greentic-messaging-tester` is a CLI that loads provider WASM components directly (no operator needed) and drives them through send, ingress, webhook, and listen operations. Use it to validate Telegram, Dummy, or any provider in isolation.

## Location

```
greentic-messaging-providers/crates/greentic-messaging-tester/
```

## Build

```bash
cd greentic-messaging-providers

# Build the tester CLI
cargo build --release --package greentic-messaging-tester

# Binary at: target/release/greentic-messaging-tester
```

Requires provider WASMs to be built first:

```bash
# Build all components
SKIP_WASM_TOOLS_VALIDATION=1 ./tools/build_components.sh

# Or build only what you need
cargo build --release --package messaging-provider-telegram --target wasm32-wasip2
cargo build --release --package messaging-provider-dummy --target wasm32-wasip2
cargo build --release --package messaging-ingress-telegram --target wasm32-wasip2
```

### How the tester finds WASMs

When you pass `--provider messaging-telegram`, the tester searches for the WASM in this order:

1. `$GREENTIC_PROVIDER_WASM` env var (if set, uses that directly)
2. `target/components/messaging-provider-telegram/component.wasm`
3. `target/wasm32-wasip2/release/messaging_provider_telegram.wasm`
4. `target/wasm32-wasip2/debug/messaging_provider_telegram.wasm`
5. `components/messaging-provider-telegram/target/wasm32-wasip2/release/...`

If nothing is found, it attempts `cargo component build -p messaging-provider-telegram` automatically.

All paths are relative to the `greentic-messaging-providers/` root.

## CLI Commands

All commands below assume you're running from the `greentic-messaging-providers/` directory.

### `requirements` -- Show provider requirements

```bash
target/release/target/release/greentic-messaging-tester requirements --provider messaging-telegram
```

Shows the secrets, config, and sample values a provider needs.

### `send` -- Send a message through a provider

```bash
target/release/greentic-messaging-tester send \
  --provider messaging-telegram \
  --values crates/greentic-messaging-tester/tests/fixtures/values/telegram.json \
  --text "Hello from tester" \
  --to 7951102355
```

This invokes the full egress pipeline: `render_plan` -> `encode` -> `send_payload`.

Output is a JSON object printed to stdout:

```json
{
  "plan": { ... },           // render_plan output
  "encode_result": {         // encode result
    "ok": true,
    "payload": { "content_type": "application/json", "body_b64": "..." }
  },
  "http_calls": [            // HTTP requests made (captured in mock mode)
    { "request": { "method": "POST", "url": "https://api.telegram.org/..." }, "response": { "status": 200 } }
  ],
  "result": { "ok": true }   // final send_payload result
}
```

With an Adaptive Card:

```bash
target/release/greentic-messaging-tester send \
  --provider messaging-telegram \
  --values crates/greentic-messaging-tester/tests/fixtures/values/telegram.json \
  --card /path/to/card.json \
  --to 7951102355
```

### `ingress` -- Process an inbound webhook

```bash
target/release/greentic-messaging-tester ingress \
  --provider messaging-telegram \
  --values crates/greentic-messaging-tester/tests/fixtures/values/telegram.json \
  --http_in /path/to/http_request.json \
  --public_base_url "https://example.com"
```

The `--http_in` file is a JSON representing an HTTP request:

```json
{
  "method": "POST",
  "path": "/telegram/webhook",
  "query": "",
  "headers": {
    "content-type": "application/json"
  },
  "body": "{\"update_id\":123,\"message\":{\"message_id\":1,\"from\":{\"id\":7951102355},\"chat\":{\"id\":7951102355},\"text\":\"hello\"}}"
}
```

| Field | Required | Description |
|-------|----------|-------------|
| `method` | yes | HTTP method (GET, POST) |
| `path` | yes | Request path |
| `query` | no | Query string |
| `headers` | no | HTTP headers as key-value object |
| `body` | no | Raw request body as string |

Invokes the provider's `ingest_http` op and prints the parsed `ChannelMessageEnvelope` events to stdout.

### `webhook` -- Register/verify webhooks

```bash
target/release/greentic-messaging-tester webhook \
  --provider messaging-telegram \
  --values crates/greentic-messaging-tester/tests/fixtures/values/webhook_telegram.json \
  --public_base_url "https://my-tunnel.trycloudflare.com"

# Dry run (show what would be called, don't make real API calls)
target/release/greentic-messaging-tester webhook \
  --provider messaging-telegram \
  --values crates/greentic-messaging-tester/tests/fixtures/values/webhook_telegram.json \
  --public_base_url "https://my-tunnel.trycloudflare.com" \
  --dry_run
```

### `listen` -- Start HTTP listener for live webhooks

```bash
target/release/greentic-messaging-tester listen \
  --provider messaging-dummy \
  --values crates/greentic-messaging-tester/tests/fixtures/values/listen_dummy.json \
  --host 127.0.0.1 \
  --port 8080 \
  --path /webhook \
  --public_base_url "http://localhost:8080"
```

Starts an HTTP server and routes incoming requests through the provider's `ingest_http` op.

## Values File Format

The values JSON configures the WASM harness with secrets, config, HTTP mocking, and state:

```json
{
  "config": {
    "api_base": "https://api.telegram.org"
  },
  "secrets": {
    "TELEGRAM_BOT_TOKEN": "your-bot-token"
  },
  "to": {
    "chat_id": "123456789",
    "channel": "telegram-cli"
  },
  "http": "mock",
  "state": {}
}
```

| Field | Type | Purpose |
|-------|------|---------|
| `config` | object | Non-secret configuration values |
| `secrets` | object | Secret values (tokens, keys) |
| `to` | object | Default destination for send |
| `http` | `"mock"` or `"live"` | `mock` captures HTTP calls, `live` makes real API calls |
| `state` | object | Initial state-store values |

### Pre-built fixtures

| File | Provider | Mode |
|------|----------|------|
| `crates/greentic-messaging-tester/tests/fixtures/values/telegram.json` | Telegram | mock (dummy token) |
| `crates/greentic-messaging-tester/tests/fixtures/values/listen_dummy.json` | Dummy | mock (no secrets) |
| `crates/greentic-messaging-tester/tests/fixtures/values/webhook_telegram.json` | Telegram webhook | mock (dummy token) |

### Live mode (real API calls)

To test against real APIs, create a values file with `"http": "live"` and real credentials:

```json
{
  "config": {
    "api_base": "https://api.telegram.org"
  },
  "secrets": {
    "TELEGRAM_BOT_TOKEN": "1234567890:AAF_real_token_here"
  },
  "to": {
    "chat_id": "7951102355"
  },
  "http": "live",
  "state": {}
}
```

```bash
target/release/greentic-messaging-tester send \
  --provider messaging-telegram \
  --values /path/to/live-telegram.json \
  --text "Live test message"
```

## Validating Telegram

### Mock mode (no API calls)

```bash
# Send (captures the HTTP request that would be made)
target/release/greentic-messaging-tester send \
  --provider messaging-telegram \
  --values crates/greentic-messaging-tester/tests/fixtures/values/telegram.json \
  --text "Hello"

# Output shows the Telegram Bot API request body:
# POST https://api.telegram.org/bot<token>/sendMessage
# {"chat_id": "123456789", "text": "Hello", "parse_mode": "HTML"}
```

### Live mode

```bash
# Create live values
cat > /tmp/telegram-live.json << 'EOF'
{
  "config": {"api_base": "https://api.telegram.org"},
  "secrets": {"TELEGRAM_BOT_TOKEN": "YOUR_BOT_TOKEN"},
  "to": {"chat_id": "YOUR_CHAT_ID"},
  "http": "live",
  "state": {}
}
EOF

# Send a message
target/release/greentic-messaging-tester send \
  --provider messaging-telegram \
  --values /tmp/telegram-live.json \
  --text "Hello from greentic-messaging-tester"
```

## Validating Dummy

The dummy provider accepts any input and returns mock responses. It needs no secrets:

```bash
# Send
target/release/greentic-messaging-tester send \
  --provider messaging-dummy \
  --values crates/greentic-messaging-tester/tests/fixtures/values/listen_dummy.json \
  --text "Test message"

# Listen (start webhook listener)
target/release/greentic-messaging-tester listen \
  --provider messaging-dummy \
  --values crates/greentic-messaging-tester/tests/fixtures/values/listen_dummy.json \
  --host 127.0.0.1 \
  --port 9090 \
  --path /webhook \
  --public_base_url "http://localhost:9090"

# In another terminal, send a webhook:
curl -X POST http://localhost:9090/webhook \
  -H "Content-Type: application/json" \
  -d '{"text": "incoming message"}'
```

## Provider Snapshot Tests (provider-tests crate)

The `provider-tests` crate provides a WASM-based test harness with snapshot testing for all providers. This is the primary automated test suite.

### Location

```
greentic-messaging-providers/crates/provider-tests/
```

### Running all provider tests

```bash
cd greentic-messaging-providers

# Run all provider tests (requires WASMs to be built)
cargo test --package provider-tests

# Run specific test suites
cargo test --package provider-tests -- provider_harness        # Snapshot tests
cargo test --package provider-tests -- provider_core_telegram  # Telegram unit tests
cargo test --package provider-tests -- provider_core_dummy     # Dummy unit tests
cargo test --package provider-tests -- instantiation           # WASM instantiation
cargo test --package provider-tests -- universal_ops           # Universal op conformance
cargo test --package provider-tests -- registry_fixtures       # Registry fixtures
```

### Test suites

| Test file | Count | Purpose |
|-----------|-------|---------|
| `provider_harness.rs` | 40+ | Snapshot tests: inbound/outbound/AC translation per provider |
| `provider_core_telegram.rs` | - | Telegram-specific unit tests |
| `provider_core_slack.rs` | - | Slack-specific unit tests |
| `provider_core_teams.rs` | - | Teams-specific unit tests |
| `provider_core_webchat.rs` | - | WebChat-specific unit tests |
| `provider_core_webex.rs` | - | Webex-specific unit tests |
| `provider_core_whatsapp.rs` | - | WhatsApp-specific unit tests |
| `provider_core_email.rs` | - | Email-specific unit tests |
| `provider_core_dummy.rs` | - | Dummy provider tests |
| `provider_ingress_components.rs` | - | Ingress webhook handler tests |
| `instantiation_providers.rs` | 8 | WASM component instantiation (all 8 providers load OK) |
| `universal_ops_conformance.rs` | 10+ | Universal operation conformance |
| `universal_ops_render_plan.rs` | - | render_plan conformance |
| `universal_ops_email.rs` | - | Email-specific ops |
| `registry_fixtures.rs` | 10 | Registry fixture consistency |

### Snapshot files

Snapshots live in `tests/snapshots/` (90+ `.snap` files):

```
provider_harness__adaptivecard_translation_snapshot_telegram__basic.snap
provider_harness__adaptivecard_translation_snapshot_slack__basic.snap
provider_harness__inbound_snapshot_telegram__text_message.snap
...
```

### Updating snapshots

When provider behavior changes intentionally:

```bash
# Install cargo-insta (first time)
cargo install cargo-insta

# Review and update snapshots interactively
cargo insta review --package provider-tests

# Or accept all changes
cargo insta accept --package provider-tests
```

### Running fixture-based tests per pack

Each pack in `packs/*/fixtures/` contains E2E test fixtures:

```
packs/messaging-telegram/fixtures/
├── requirements.expected.json      # Expected requirements output
├── setup.input.json                # Setup flow input
├── setup.expected.plan.json        # Expected setup plan
├── egress.request.json             # Egress test input
├── egress.expected.summary.json    # Expected egress summary
├── ingress.request.json            # Ingress test input
└── ingress.expected.message.json   # Expected ingress output
```

These are validated by:

```bash
# Validate all pack fixtures
python3 tools/validate_pack_fixtures.py

# Regenerate fixtures from current behavior
./tools/regenerate_registry_fixtures.sh
```

## Full Test Pipeline

Run everything in order:

```bash
cd greentic-messaging-providers

# 1. Build WASMs
SKIP_WASM_TOOLS_VALIDATION=1 ./tools/build_components.sh

# 2. Run all tests (library + provider + snapshot)
cargo test --workspace

# 3. Run clippy
cargo clippy --workspace -- -D warnings

# 4. Check formatting
cargo fmt --all --check
```

### Test count summary

| Suite | Count |
|-------|-------|
| Provider snapshots | 40+ |
| Provider unit tests | 30+ |
| WASM instantiation | 8 |
| Universal ops | 15+ |
| Registry fixtures | 10 |
| CardKit smoke | 5+ |
| Tester integration | 3 |
| **Total** | ~110+ |
