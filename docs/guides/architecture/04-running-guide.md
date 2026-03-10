# Running Guide

## Prerequisites

### Rust Toolchain
```bash
# Rust 1.90+ required (MSRV)
rustc --version

# Add WASM target
rustup target add wasm32-wasip2

# Install cargo-component
cargo install cargo-component --locked

# Install cargo-binstall (for easy binary installs)
cargo install cargo-binstall --locked

# Install greentic-pack CLI
cargo binstall greentic-pack --no-confirm --locked
```

### Other Tools
- `jq` - JSON processing (used by build scripts)
- `python3` (3.11+ for `tomllib`) - version extraction in CI scripts

### NOT Required (for embedded demo mode)
- NATS server (default: `--nats=off`)
- cloudflared (only needed for external webhook tunnels)
- Redis (in-memory backends used in demo)

---

## Step 1: Build Operator Binary

```bash
cd /root/works/personal/greentic/greentic-operator
cargo build --release
# Binary: target/release/greentic-operator
```

---

## Step 2: Build Provider WASM Components + Packs

```bash
cd /root/works/personal/greentic/greentic-messaging-providers

# 2a. Build all WASM components
./scripts/build-provider-wasms.sh
# Output: dist/wasms/*.wasm

# 2b. Build .gtpack archives
DRY_RUN=1 ./tools/build_packs_only.sh
# Output: dist/packs/*.gtpack
```

### Manual single-pack build (alternative)

```bash
# Build one component
cargo component build --release --target wasm32-wasip2 \
  -p messaging-provider-telegram

# Build one pack
greentic-pack build \
  --in packs/messaging-telegram \
  --gtpack-out dist/packs/messaging-telegram.gtpack
```

---

## Step 3: Create Demo Bundle

```bash
cd /root/works/personal/greentic
OPERATOR=./greentic-operator/target/release/greentic-operator

# 3a. Scaffold empty bundle
$OPERATOR demo new demo-bundle

# 3b. Copy provider packs into bundle
cp greentic-messaging-providers/dist/packs/messaging-telegram.gtpack \
   demo-bundle/providers/messaging/
cp greentic-messaging-providers/dist/packs/messaging-webex.gtpack \
   demo-bundle/providers/messaging/
cp greentic-messaging-providers/dist/packs/messaging-webchat.gtpack \
   demo-bundle/providers/messaging/
```

Bundle structure:
```
demo-bundle/
  greentic.demo.yaml        # bundle marker
  providers/
    messaging/               # drop .gtpack files here
      messaging-telegram.gtpack
      messaging-webex.gtpack
      messaging-webchat.gtpack
    events/                  # event providers (if any)
  packs/                     # app packs
  tenants/
    default/
      tenant.gmap            # access policies
  state/
  resolved/
  logs/
```

---

## Step 4: Setup (Interactive Provider Config)

```bash
# Setup all providers
$OPERATOR demo setup \
  --bundle demo-bundle \
  --tenant default

# Setup specific provider only
$OPERATOR demo setup \
  --bundle demo-bundle \
  --tenant default \
  --provider telegram

# Non-interactive (pre-supply answers)
$OPERATOR demo setup \
  --bundle demo-bundle \
  --tenant default \
  --setup-input answers.yaml
```

### What setup does:
1. Discovers all .gtpack files in `providers/messaging/`
2. For each provider, runs the `setup_default` flow
3. Flow: emit_questions → collect answers → validate → apply (write config/secrets)
4. Writes results to `state/runtime/<tenant>/providers/<pack>/`

### Setup answers file format (answers.yaml):
```yaml
telegram:
  public_base_url: "https://xxxx.trycloudflare.com"
  bot_token: "123456:ABC-DEF"
  default_chat_id: "-100123456"
webex:
  public_base_url: "https://xxxx.trycloudflare.com"
  bot_token: "YOUR_WEBEX_BOT_TOKEN"
webchat:
  public_base_url: "http://localhost:7878"
```

---

## Step 5: Build Resolved Bundle

```bash
$OPERATOR demo build \
  --bundle demo-bundle \
  --tenant default
```

This validates packs and creates `resolved/<tenant>.yaml` manifests.

---

## Step 6: Start (Foreground)

```bash
# Default embedded mode (no NATS needed, cloudflared auto-starts)
GREENTIC_ENV=dev $OPERATOR demo start \
  --bundle demo-bundle \
  --tenant default \
  --env dev

# Without cloudflared tunnel
GREENTIC_ENV=dev $OPERATOR demo start \
  --bundle demo-bundle \
  --tenant default \
  --env dev \
  --cloudflared=off

# With external NATS
GREENTIC_ENV=dev $OPERATOR demo start \
  --bundle demo-bundle \
  --tenant default \
  --env dev \
  --nats=external --nats-url nats://127.0.0.1:4222
```

Services started:
- HTTP ingress: `http://127.0.0.1:8080`
- Cloudflared tunnel: check `demo-bundle/logs/cloudflared.log` for URL
- Webhook route: `/v1/messaging/ingress/{provider}/{tenant}/{team}`
- Direct Line: `/v3/directline/*` and `/token`

Press `Ctrl+C` to stop.

---

## Step 7: Test (Separate Terminal)

### Send a message
```bash
# Telegram
GREENTIC_ENV=dev $OPERATOR demo send \
  --bundle demo-bundle \
  --provider messaging-telegram \
  --to "7951102355" \
  --text "Hello from Greentic" \
  --tenant default

# Webex (use room ID)
GREENTIC_ENV=dev $OPERATOR demo send \
  --bundle demo-bundle \
  --provider messaging-webex \
  --to "Y2lzY29zcGFyazovL..." \
  --text "Hello from Greentic" \
  --tenant default

# Slack
GREENTIC_ENV=dev $OPERATOR demo send \
  --bundle demo-bundle \
  --provider messaging-slack \
  --to "C0AFWP5C067" \
  --text "Hello from Greentic" \
  --tenant default

# Email
GREENTIC_ENV=dev $OPERATOR demo send \
  --bundle demo-bundle \
  --provider messaging-email \
  --to "user@example.com" \
  --text "Hello from Greentic" \
  --tenant default
```

### Test ingress (synthetic webhook via HTTP)
```bash
# Telegram (full round-trip: ingress → echo bot → send reply)
curl -X POST http://localhost:8080/v1/messaging/ingress/messaging-telegram/default/default \
  -H "Content-Type: application/json" \
  -d '{"update_id":1,"message":{"message_id":1,"from":{"id":123,"is_bot":false,"first_name":"T"},"chat":{"id":123,"type":"private"},"date":1,"text":"hello"}}'

# Slack
curl -X POST http://localhost:8080/v1/messaging/ingress/messaging-slack/default/default \
  -H "Content-Type: application/json" \
  -d '{"type":"event_callback","event":{"type":"message","channel":"C0AFWP5C067","user":"U123","text":"hello","ts":"1.0"}}'

# WebChat (Direct Line protocol)
TOKEN=$(curl -s -X POST http://localhost:8080/token | jq -r .token)
CONV=$(curl -s -X POST http://localhost:8080/v3/directline/conversations -H "Authorization: Bearer $TOKEN")
CONV_ID=$(echo $CONV | jq -r .conversationId)
CONV_TOKEN=$(echo $CONV | jq -r .token)
curl -X POST "http://localhost:8080/v3/directline/conversations/$CONV_ID/activities" \
  -H "Authorization: Bearer $CONV_TOKEN" -H "Content-Type: application/json" \
  -d '{"type":"message","text":"hello","from":{"id":"user1"}}'
# Wait 2s then GET activities to see echo bot reply
curl "http://localhost:8080/v3/directline/conversations/$CONV_ID/activities" \
  -H "Authorization: Bearer $CONV_TOKEN"
```

### WebChat SPA (browser)
```bash
cd greentic-webchat && npm run dev
# Open: http://localhost:5173/dev?directline=http://localhost:8080
```

### List available packs/flows
```bash
$OPERATOR demo list-packs \
  --bundle demo-bundle \
  --domain messaging

$OPERATOR demo list-flows \
  --bundle demo-bundle \
  --pack messaging-telegram \
  --domain messaging
```

---

## Access Policies (gmap)

```bash
# Allow a provider
$OPERATOR demo allow \
  --bundle demo-bundle \
  --tenant default \
  messaging-telegram

# Forbid a specific flow
$OPERATOR demo forbid \
  --bundle demo-bundle \
  --tenant default \
  messaging-telegram/some_flow

# Manual edit: demo-bundle/tenants/default/tenant.gmap
# Format: PACK[/FLOW[/NODE]] = public|forbidden
```

---

## Useful Debug Commands

```bash
# Validate pack health
greentic-pack doctor --validate demo-bundle/providers/messaging/messaging-telegram.gtpack

# Provider diagnostics
$OPERATOR demo diagnostics \
  --bundle demo-bundle \
  --provider telegram

# Check secrets status
$OPERATOR demo secrets list \
  --bundle demo-bundle \
  --tenant default

# Full operator doctor
$OPERATOR demo doctor \
  --bundle demo-bundle
```

---

## CI / Local Check Scripts

### Operator
```bash
cd /root/works/personal/greentic/greentic-operator
./ci/local_check.sh    # fmt + clippy + test + package
```

### Messaging Providers
```bash
cd /root/works/personal/greentic/greentic-messaging-providers
./ci/local_check.sh    # full pipeline: fmt → clippy → build wasms → gen flows → sync packs → doctor → test
```

---

## Environment Variables

| Variable | Purpose | Default |
|----------|---------|---------|
| `PACK_VERSION` | Override pack version | From Cargo.toml |
| `DRY_RUN` | `1` = build only, no OCI push | - |
| `PACKC_BUILD_FLAGS` | Extra flags for `greentic-pack build` | - |
| `GREENTIC_ENV` | Secret namespace | `demo` |
| `GREENTIC_OPERATOR_SKIP_DOCTOR` | Skip doctor validation | - |
