# Testing Guide — Messaging Providers

This document covers how to build, test, and validate the messaging providers
end-to-end through the `greentic-operator` demo pipeline.

## Prerequisites

| Tool | Version | Install |
|------|---------|---------|
| Rust toolchain | 1.90+ | `rustup update` |
| `wasm32-wasip2` target | — | `rustup target add wasm32-wasip2` |
| `greentic-operator` | 0.4.23+ | `cargo binstall greentic-operator` |
| `seed-secret` | local | `cargo build --release -p seed-secret` (in `tools/seed-secret/`) |
| `zip` | any | `apt install zip` |

## 1. Unit Tests

Run the full workspace test suite:

```bash
cd greentic-messaging-providers
cargo test --workspace
```

Expected: **287 passed, 0 failed, 2 ignored** (the 2 ignored are pre-existing `pack_doctor` tests).

### Per-crate breakdown

| Crate | Tests | Notes |
|-------|-------|-------|
| `messaging-provider-dummy` | 8 | QA ops + send |
| `messaging-provider-telegram` | 8 | QA ops + send |
| `messaging-provider-slack` | 8 | QA ops + send |
| `messaging-provider-teams` | 8 | QA ops + send |
| `messaging-provider-webex` | 8 | QA ops + send |
| `messaging-provider-webchat` | 8+3 | QA ops + send + integration |
| `messaging-provider-whatsapp` | 8 | QA ops + send |
| `messaging-provider-email` | 8 | QA ops + send |
| `greentic-messaging-renderer` | 35 | 12 ac_extract + 14 planner + 5 downsample + 4 noop |
| `provider-common` | misc | Shared utilities |

Run a single provider's tests:

```bash
cargo test -p messaging-provider-slack
```

## 2. WASM Build

Build all 8 provider WASMs:

```bash
SKIP_WASM_TOOLS_VALIDATION=1 ./tools/build_components.sh
```

Output lands in `target/components/`:

```
target/components/messaging-provider-dummy.wasm
target/components/messaging-provider-email.wasm
target/components/messaging-provider-slack.wasm
target/components/messaging-provider-teams.wasm
target/components/messaging-provider-telegram.wasm
target/components/messaging-provider-webchat.wasm
target/components/messaging-provider-webex.wasm
target/components/messaging-provider-whatsapp.wasm
```

Build a single provider:

```bash
SKIP_WASM_TOOLS_VALIDATION=1 ./tools/build_component_one.sh messaging-provider-slack
```

### Build notes

- The build script uses `cargo build --target wasm32-wasip2` (not `cargo component build`).
- `cargo-component` 0.21 has a WIT resolution bug that can't find `provider-schema-core` in deps.
  Standard `cargo build` with `wit-bindgen` resolves deps correctly.
- `SKIP_WASM_TOOLS_VALIDATION=1` skips `wasm-tools validate` (avoids needing `wasm-tools` installed).

## 3. Update Demo Bundle gtpacks

After building fresh WASMs, replace them inside the demo-bundle gtpack zip files:

```bash
DEMO_BUNDLE="/root/works/personal/greentic/demo-bundle"
WASM_DIR="target/components"

for provider in dummy email slack teams telegram webchat webex whatsapp; do
  gtpack="${DEMO_BUNDLE}/providers/messaging/messaging-${provider}.gtpack"
  wasm="${WASM_DIR}/messaging-provider-${provider}.wasm"

  if [ ! -f "$gtpack" ] || [ ! -f "$wasm" ]; then
    echo "SKIP: missing $gtpack or $wasm"
    continue
  fi

  # Find the internal path of the WASM inside the zip
  wasm_entry=$(unzip -l "$gtpack" | grep "messaging-provider-${provider}.wasm" | awk '{print $4}')
  if [ -z "$wasm_entry" ]; then
    echo "SKIP: no WASM entry in $gtpack"
    continue
  fi

  # Replace: copy WASM to temp dir matching the internal path, then zip -j
  tmpdir=$(mktemp -d)
  internal_dir=$(dirname "$wasm_entry")
  mkdir -p "${tmpdir}/${internal_dir}"
  cp "$wasm" "${tmpdir}/${wasm_entry}"
  (cd "$tmpdir" && zip -u "$gtpack" "$wasm_entry")
  rm -rf "$tmpdir"

  echo "Updated: $gtpack"
done
```

## 4. Seed Provider Secrets

Secrets are stored in the encrypted dev secrets file at
`demo-bundle/.greentic/dev/.dev.secrets.env`.

Seed a secret using the `seed-secret` tool:

```bash
SEED_SECRET="/root/works/personal/greentic/tools/seed-secret/target/release/seed-secret"
SECRETS_FILE="/root/works/personal/greentic/demo-bundle/.greentic/dev/.dev.secrets.env"

# Slack bot token
$SEED_SECRET "$SECRETS_FILE" \
  "secrets://dev/default/_/messaging-slack/slack_bot_token" \
  "<your-slack-bot-token>"

# Telegram bot token
$SEED_SECRET "$SECRETS_FILE" \
  "secrets://dev/default/_/messaging-telegram/bot_token" \
  "<your-telegram-bot-token>"

# Webex bot token
$SEED_SECRET "$SECRETS_FILE" \
  "secrets://dev/default/_/messaging-webex/WEBEX_BOT_TOKEN" \
  "<your-webex-bot-token>"

# Teams (MS Graph)
$SEED_SECRET "$SECRETS_FILE" \
  "secrets://dev/default/_/messaging-teams/MS_GRAPH_TENANT_ID" \
  "<your-tenant-id>"
$SEED_SECRET "$SECRETS_FILE" \
  "secrets://dev/default/_/messaging-teams/MS_GRAPH_CLIENT_ID" \
  "<your-client-id>"
$SEED_SECRET "$SECRETS_FILE" \
  "secrets://dev/default/_/messaging-teams/MS_GRAPH_CLIENT_SECRET" \
  "<your-client-secret>"
```

## 5. E2E Tests via Operator

### Environment

All operator commands require:

```bash
export GREENTIC_ENV=dev
```

### 5.1 Slack — Send Text

```bash
GREENTIC_ENV=dev greentic-operator demo send \
  --bundle /root/works/personal/greentic/demo-bundle \
  --provider messaging-slack \
  --to "C0AFWP5C067" \
  --text "Hello from Greentic operator" \
  --tenant default --env dev
```

Expected output:
```json
{"ok":true,"message_id":"...","ts":"..."}
```

Verify: message appears in Slack channel `#C0AFWP5C067`.

### 5.2 Telegram — Send Text

```bash
GREENTIC_ENV=dev greentic-operator demo send \
  --bundle /root/works/personal/greentic/demo-bundle \
  --provider messaging-telegram \
  --to "7951102355" \
  --text "Hello from Greentic operator" \
  --tenant default --env dev
```

Expected output:
```json
{"ok":true,"message_id":"..."}
```

Verify: message appears in Telegram chat.

### 5.3 Webex — Send Text

```bash
GREENTIC_ENV=dev greentic-operator demo send \
  --bundle /root/works/personal/greentic/demo-bundle \
  --provider messaging-webex \
  --to "<email-or-room-id>" \
  --text "Hello from Greentic operator" \
  --tenant default --env dev
```

Requires a valid `WEBEX_BOT_TOKEN` seeded in the secrets file.

### 5.4 Webex — Send Adaptive Card

Create a test card file:

```bash
cat > /tmp/test-card.json << 'EOF'
{
  "type": "AdaptiveCard",
  "version": "1.3",
  "body": [
    {"type": "TextBlock", "text": "Greentic Demo", "weight": "Bolder", "size": "Large"},
    {"type": "TextBlock", "text": "This is an Adaptive Card sent via WebEx"}
  ],
  "actions": [
    {"type": "Action.OpenUrl", "title": "Visit Greentic", "url": "https://greentic.ai"}
  ]
}
EOF
```

```bash
GREENTIC_ENV=dev greentic-operator demo send \
  --bundle /root/works/personal/greentic/demo-bundle \
  --provider messaging-webex \
  --to "<email-or-room-id>" \
  --text "AC Demo" \
  --card /tmp/test-card.json \
  --tenant default --env dev
```

Verify: Adaptive Card renders natively in Webex client.

### 5.5 Dummy — Send (Dry Run)

The dummy provider always succeeds without external calls — useful for pipeline validation:

```bash
GREENTIC_ENV=dev greentic-operator demo send \
  --bundle /root/works/personal/greentic/demo-bundle \
  --provider messaging-dummy \
  --to "test" \
  --text "Pipeline validation" \
  --tenant default --env dev
```

### 5.6 WebChat

WebChat is **client-initiated** (Azure Direct Line API). `demo send` validates the
pipeline doesn't crash, but messages go to a local state store queue — no external delivery.

For a full WebChat demo, you need:
1. Start operator HTTP server: `GREENTIC_ENV=dev greentic-operator demo start --bundle demo-bundle`
2. Seed `jwt_signing_key` for token generation
3. Point `greentic-webchat` frontend at the operator

## 6. Operator Send Pipeline

The `demo send` command exercises this pipeline for each provider:

```
render_plan → encode → send_payload
```

| Step | Input | Output |
|------|-------|--------|
| `render_plan` | text + optional AC card | `RenderPlan` with body, metadata |
| `encode` | `RenderPlan` | `EncodedPayload` (provider-specific HTTP body) |
| `send_payload` | `EncodedPayload` | HTTP call to provider API |

The pipeline runs through the `schema-core-api@1.0.0` `invoke()` export, which the
operator v0.4.x uses to dispatch operations to provider WASMs.

## 7. Dual Interface Export

All 8 providers export **two** interfaces for backward/forward compatibility:

| Interface | Version | Encoding | Used by |
|-----------|---------|----------|---------|
| `greentic:component@0.6.1` | v0.6 | CBOR | Future operator (v0.5+) |
| `greentic:provider-schema-core/schema-core-api@1.0.0` | v0.4 | JSON | Current operator (v0.4.23) |

The `schema-core-api` `invoke()` function delegates to the same handlers as the v0.6
`runtime` interface, with JSON ↔ CBOR translation where needed.

## 8. Known Issues

| Issue | Impact | Workaround |
|-------|--------|------------|
| `cargo component build` can't find `provider-schema-core` | Build fails | Use `cargo build` (build script already updated) |
| Teams encode uses `message.channel` as fallback | May send to wrong destination | Needs fix similar to Slack encode fix |
| `demo setup` broken (flow engine mismatch) | Can't run interactive setup | Use `demo send` for validation |
| `greentic-pack build` broken (state-store mismatch) | Can't rebuild packs from scratch | Replace WASM inside existing gtpack zips |
| WebChat needs full HTTP server for real demo | `demo send` only validates pipeline | Use `demo start` + frontend |
| Pre-existing clippy errors in `greentic-messaging-renderer` | 5 `collapsible_if` warnings | Not related to our changes |

## 9. Troubleshooting

### "Secret not found" errors

Make sure `GREENTIC_ENV=dev` is set. The secrets backend only resolves `dev` or `test` environments.

### Messages not arriving

1. Check that `--to` is the actual provider destination (Slack channel ID, Telegram chat ID),
   **not** the Greentic channel name like `messaging.slack`.
2. Verify the bot token is valid by sending directly via the provider API (e.g., `curl`).
3. Check operator output for `"ok": false` and error details.

### WASM build errors

If `wasm-tools validate` fails, set `SKIP_WASM_TOOLS_VALIDATION=1`.
If `cargo build` can't find WIT deps, make sure each provider's `wit/<provider>/deps/`
directory contains the `provider-schema-core/package.wit` file.

### Docker `credsStore` errors

If `oras` or Docker commands fail with credential errors, remove or rename
`~/.docker/config.json` (it may contain `"credsStore": "desktop.exe"` from Windows).
