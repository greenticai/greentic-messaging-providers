# Testing Guide — Messaging Providers

This document covers how to build, test, and validate the messaging providers
end-to-end through the `greentic-operator` demo pipeline.

## Prerequisites

| Tool | Version | Install |
|------|---------|---------|
| Rust toolchain | 1.90+ | `rustup update` |
| `wasm32-wasip2` target | — | `rustup target add wasm32-wasip2` |
| `greentic-operator` | 0.4.23+ | `cargo binstall greentic-operator` |
| `greentic-secrets` | 0.4.x+ | `cargo binstall greentic-secrets` or build from `greentic-secrets/` |
| `zip` | any | `apt install zip` |

## 1. Unit Tests

Run the full workspace test suite:

```bash
cd greentic-messaging-providers
cargo test --workspace
```

Expected: **329 passed, 0 failed, 2 ignored** (the 2 ignored are pre-existing `pack_doctor` tests).

### Per-crate breakdown

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

Run a single provider's tests:

```bash
cargo test -p messaging-provider-slack
```

## 1b. QA-Specific Tests

### Unit Tests (provider-common)

The `qa_invoke_bridge` module has 5 unit tests covering the JSON↔CBOR bridge:

```bash
cargo test -p provider-common qa_invoke_bridge
```

Tests: `extract_mode_parses_setup`, `extract_mode_defaults_to_setup`,
`dispatch_returns_none_for_unknown_op`, `dispatch_qa_spec_returns_json`,
`dispatch_apply_answers_bridges_json_cbor`, `dispatch_i18n_keys_returns_json_array`.

### Per-Provider QA Tests (standard_provider_tests!)

Each provider has generated tests from the `standard_provider_tests!` macro:

```bash
# Run QA tests for a single provider
cargo test -p messaging-provider-slack qa
```

Tests per provider:
- `qa_spec_returns_questions_for_setup` — verifies qa-spec returns questions with i18n keys
- `apply_answers_setup_returns_valid_config` — validates apply-answers round-trip
- `apply_answers_remove_returns_remove_plan` — checks remove mode
- `apply_answers_validation_rejects_invalid` — confirms validation errors
- `schema_hash_is_stable` — ensures describe payload hash matches expected

### WASM Integration Tests (provider-tests)

Three integration tests verify the QA ops work through the full WASM → schema-core-api
invoke() → qa_invoke_bridge → provider pipeline:

```bash
cargo test -p provider-tests -- qa_spec_via_invoke
cargo test -p provider-tests -- apply_answers_via_invoke
cargo test -p provider-tests -- i18n_keys_via_invoke
```

These instantiate real WASM components via Wasmtime and call invoke() with JSON, exactly
as the operator does. All 8 providers (including Dummy) are tested.

### E2E QA via Operator

The operator's `demo setup` command exercises the full QA flow end-to-end:

```bash
GREENTIC_ENV=dev greentic-operator demo setup \
  --bundle demo-bundle \
  --domain messaging \
  --provider messaging-slack
```

This runs the complete QA contract:
1. `supports_component_qa_contract()` — checks pack manifest for QA ops
2. `invoke("qa-spec", {"mode":"setup"})` — gets question list
3. `invoke("i18n-keys", {})` — gets localization keys, validates against qa-spec
4. `invoke("apply-answers", {...})` — validates answers, returns config
5. `validate_config_strict()` — validates config against JSON schema

**Note:** For `demo setup` to work, the gtpack's `manifest.cbor` must declare the
QA ops (`qa-spec`, `apply-answers`, `i18n-keys`). See "Update manifest.cbor in gtpacks"
below.

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

Seed a secret using `greentic-secrets apply` (or the legacy `seed-secret` tool if
available). Secrets are written to the encrypted dev secrets file.

```bash
SECRETS_FILE="/root/works/personal/greentic/demo-bundle/.greentic/dev/.dev.secrets.env"

# Slack bot token
greentic-secrets apply "$SECRETS_FILE" \
  "secrets://dev/default/_/messaging-slack/slack_bot_token" \
  "<your-slack-bot-token>"

# Telegram bot token
greentic-secrets apply "$SECRETS_FILE" \
  "secrets://dev/default/_/messaging-telegram/bot_token" \
  "<your-telegram-bot-token>"

# Webex bot token
greentic-secrets apply "$SECRETS_FILE" \
  "secrets://dev/default/_/messaging-webex/WEBEX_BOT_TOKEN" \
  "<your-webex-bot-token>"

# Teams (MS Graph) — Public client (refresh_token grant)
# NOTE: Do NOT seed MS_GRAPH_CLIENT_SECRET for public client apps.
# Azure public clients must not include a client_secret with the refresh_token flow.
greentic-secrets apply "$SECRETS_FILE" \
  "secrets://dev/demo/_/messaging-teams/MS_GRAPH_TENANT_ID" \
  "<your-tenant-id>"
greentic-secrets apply "$SECRETS_FILE" \
  "secrets://dev/demo/_/messaging-teams/MS_GRAPH_CLIENT_ID" \
  "<your-client-id>"
greentic-secrets apply "$SECRETS_FILE" \
  "secrets://dev/demo/_/messaging-teams/MS_GRAPH_REFRESH_TOKEN" \
  "<your-refresh-token>"
```

For Teams with a **confidential client** (web app with secret), replace
`MS_GRAPH_REFRESH_TOKEN` with `MS_GRAPH_CLIENT_SECRET`.

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

### 5.7 Teams — Send Text

```bash
GREENTIC_ENV=dev greentic-operator demo send \
  --bundle /root/works/personal/greentic/demo-bundle \
  --provider messaging-teams \
  --to "c3392cbc-2cb0-48e8-9247-504d8defea40:19:wQzzrth6t3YA-aEdLzt8Pse3kW3Us-nJl9XzN-5NcEE1@thread.tacv2" \
  --text "Hello from Greentic operator" \
  --tenant demo --env dev
```

Expected output:
```json
{"ok":true,"message_id":"..."}
```

Verify: message appears in the Teams channel. The `--to` format is `{team_id}:{channel_id}`.

Note: Teams secrets must be seeded under tenant `demo` (not `default`).

### 5.8 Teams — Ingress (CLI)

Create a sample Teams webhook payload:

```bash
cat > /tmp/teams-webhook.json << 'EOF'
{
  "type": "message",
  "text": "Hello from webhook test",
  "from": { "id": "user123", "name": "Test User" },
  "channelData": {
    "team": { "id": "c3392cbc-2cb0-48e8-9247-504d8defea40" },
    "channel": { "id": "19:wQzzrth6t3YA-aEdLzt8Pse3kW3Us-nJl9XzN-5NcEE1@thread.tacv2" }
  }
}
EOF
```

```bash
GREENTIC_ENV=dev greentic-operator demo ingress \
  --bundle /root/works/personal/greentic/demo-bundle \
  --provider messaging-teams \
  --tenant demo \
  --body /tmp/teams-webhook.json
```

Expected: `events[0].to` contains `[{id: "c3392cbc-...:19:...@thread.tacv2", kind: "channel"}]`.

### 5.9 Teams — Ingress via Operator HTTP

```bash
# Start the operator HTTP server
GREENTIC_ENV=dev greentic-operator demo start \
  --bundle /root/works/personal/greentic/demo-bundle \
  --cloudflared off --nats off --skip-setup --skip-secrets-init --domains messaging

# POST the webhook payload
curl -X POST http://localhost:8080/messaging/ingress/messaging-teams/demo/default \
  -H "Content-Type: application/json" \
  -d @/tmp/teams-webhook.json
```

Expected: HTTP 200 with `{"ok": true, ...}` and operator logs show `dispatch_egress`.

### 5.10 Teams — Full Round-Trip

1. Start operator: see 5.9 above.
2. POST a Teams webhook payload to the ingress endpoint.
3. The operator dispatches egress with the envelope's `to` field populated.
4. The egress pipeline calls `send_payload` which sends a reply via the Graph API.
5. Verify: reply message appears in the Teams channel.

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

### QA Operations via invoke()

QA ops (`qa-spec`, `apply-answers`, `i18n-keys`) are dispatched through the same
`invoke()` path. The `qa_invoke_bridge` module in `provider-common` handles the
JSON→CBOR→JSON round-trip:

```
invoke("qa-spec", {"mode":"setup"})        → questions list (JSON)
invoke("apply-answers", {"mode":"setup",
  "answers":{...}, "current_config":{...}}) → {"ok":true, "config":{...}}
invoke("i18n-keys", {})                    → ["key1", "key2", ...]
```

## 7b. Update manifest.cbor in gtpacks (for QA E2E)

The operator checks `manifest.cbor` inside the gtpack for QA op declarations. If you've
added QA ops to `pack.manifest.json` but the gtpack's `manifest.cbor` is stale, the
operator won't detect QA support.

To update, either:
1. Rebuild packs with `greentic-pack build` (if working)
2. Or regenerate `manifest.cbor` from `pack.manifest.json` and replace in the zip:

```bash
# Convert pack.manifest.json to CBOR and update gtpack
for provider in slack teams telegram email webex whatsapp webchat dummy; do
  manifest_json="packs/messaging-${provider}/pack.manifest.json"
  gtpack="packs/messaging-${provider}/dist/messaging-${provider}.gtpack"
  [ ! -f "$manifest_json" ] || [ ! -f "$gtpack" ] && continue

  tmpdir=$(mktemp -d)
  # Use a small script or tool to convert JSON → CBOR
  # Then: (cd "$tmpdir" && zip -u "$gtpack" manifest.cbor)
  rm -rf "$tmpdir"
done
```

**Note:** This requires a JSON-to-CBOR conversion tool. The `greentic-messaging-packgen`
crate generates both formats during pack builds.

## 8. Known Issues

| Issue | Impact | Workaround |
|-------|--------|------------|
| `cargo component build` can't find `provider-schema-core` | Build fails | Use `cargo build` (build script already updated) |
| `demo setup` may need manifest.cbor update | QA ops not detected | Regenerate manifest.cbor in gtpack (see section 7b) |
| `greentic-pack build` broken (state-store mismatch) | Can't rebuild packs from scratch | Replace WASM inside existing gtpack zips |
| WebChat needs full HTTP server for real demo | `demo send` only validates pipeline | Use `demo start` + frontend |
| Teams Azure public client must not send `client_secret` | Auth fails with 400 | Only seed `refresh_token`, not `client_secret` |
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
