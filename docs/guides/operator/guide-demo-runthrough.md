# Full Demo Run-Through

Complete step-by-step guide to run the Greentic platform demo from scratch. Covers all capabilities: messaging providers, telemetry (OTel), and state (Redis).

## What You'll Demo

| Feature | What It Shows |
|---------|---------------|
| Multi-provider messaging | Send/receive via Telegram, Slack, Teams, WebChat |
| Telemetry capability | Auto-configured OTel pipeline from WASM component |
| State capability | Redis-backed persistent KV store |
| Operation subscriptions | Structured spans for every provider operation |
| Capability auto-discovery | Platform dynamically extended via gtpack plugins |

---

## Step 0: Prerequisites

### Software

```bash
# Check operator is installed
greentic-operator --version  # Needs v0.4.32+

# Check Rust (for building packs)
rustc --version  # 1.89+

# Check Docker (for Redis + Jaeger)
docker --version
```

### Credentials (for live messaging)

| Provider | What You Need | Where to Get |
|----------|---------------|--------------|
| Telegram | Bot token | @BotFather on Telegram |
| Slack | Bot token | Slack API → OAuth & Permissions |
| Teams | Tenant ID + Client ID + Refresh token | Azure AD app registration |
| WebChat | Nothing (built-in Direct Line) | — |

You can demo with WebChat only (zero external credentials needed).

---

## Step 1: Prepare Demo Bundle

### 1.1 Clone and Setup

```bash
cd /root/works/personal/greentic
git submodule update --init greentic-operator greentic-messaging-providers greentic-types
```

### 1.2 Check Demo Bundle Exists

```bash
ls demo-bundle/
# Should have: greentic.demo.yaml, providers/, state/, .greentic/
```

If no `demo-bundle/`, create one:

```bash
gtc op demo new demo-bundle --out .
mkdir -p demo-bundle/providers/messaging
```

### 1.3 Copy Messaging Provider Packs

```bash
# Copy pre-built gtpacks (if available in dist/)
cp greentic-messaging-providers/dist/packs/messaging-*.gtpack demo-bundle/providers/messaging/

# Or verify they already exist
ls demo-bundle/providers/messaging/
# messaging-telegram.gtpack, messaging-slack.gtpack, messaging-webchat.gtpack, etc.
```

---

## Step 2: Build Capability Packs

### 2.1 Build the Pack Tool (one time)

```bash
cd tools/build-capability-pack
cargo build
cd ../..
```

### 2.2 Build Telemetry Pack

```bash
./tools/build-capability-pack/target/debug/build-capability-pack \
  packs/telemetry-otlp \
  demo-bundle/providers/messaging/telemetry-otlp.gtpack
```

### 2.3 Build State Packs

```bash
# Redis (persistent state)
./tools/build-capability-pack/target/debug/build-capability-pack \
  greentic-messaging-providers/packs/state-redis \
  demo-bundle/providers/messaging/state-redis.gtpack

# Memory (ephemeral fallback)
./tools/build-capability-pack/target/debug/build-capability-pack \
  greentic-messaging-providers/packs/state-memory \
  demo-bundle/providers/messaging/state-memory.gtpack
```

### 2.4 Create Install Records

```bash
mkdir -p demo-bundle/state/runtime/demo/default/capabilities

# Telemetry
cat > demo-bundle/state/runtime/demo/default/capabilities/telemetry-otlp-v1.install.json << 'EOF'
{
  "cap_id": "greentic.cap.telemetry.v1",
  "stable_id": "telemetry-otlp-v1",
  "pack_id": "telemetry-otlp",
  "status": "ready",
  "config_state_keys": [],
  "timestamp_unix_sec": 1741035600
}
EOF

# State Redis
cat > demo-bundle/state/runtime/demo/default/capabilities/state.redis.kv.01.install.json << 'EOF'
{
  "cap_id": "greentic.cap.state.kv.v1",
  "stable_id": "state.redis.kv.01",
  "pack_id": "state-redis",
  "status": "ready",
  "config_state_keys": [],
  "timestamp_unix_sec": 1741035600
}
EOF
```

### 2.5 Verify Bundle

```bash
ls demo-bundle/providers/messaging/
```

Expected:
```
messaging-dummy.gtpack
messaging-email.gtpack
messaging-slack.gtpack
messaging-teams.gtpack
messaging-telegram.gtpack
messaging-webchat.gtpack
messaging-webex.gtpack
messaging-whatsapp.gtpack
state-memory.gtpack
state-redis.gtpack
telemetry-otlp.gtpack
```

---

## Step 3: Start Infrastructure

### 3.1 Redis (for state persistence)

```bash
docker run -d --name redis -p 6379:6379 redis:latest

# Verify
docker exec redis redis-cli ping  # → PONG
```

### 3.2 Jaeger (optional, for visual traces)

```bash
docker run -d --name jaeger \
  -p 16686:16686 \
  -p 4317:4317 \
  jaegertracing/all-in-one:latest
```

---

## Step 4: Start the Demo

### Option A: With Telemetry (stdout)

```bash
GREENTIC_ENV=dev gtc op demo start --bundle demo-bundle
```

### Option B: With Telemetry (Jaeger)

```bash
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317 \
GREENTIC_ENV=dev gtc op demo start --bundle demo-bundle
```

### Expected Log Output

```
Public URL (service=cloudflared): https://xxxx.trycloudflare.com

Started services:
cloudflared (pid=...) [url=https://xxxx.trycloudflare.com | log=...]

secrets runner ctx: ... provider_id=telemetry.configurator pack_id=telemetry-otlp ...

state.capability: offer=state.redis.kv.01 pack=state-redis priority=50
state backend: connected to Redis (url=redis://127.0.0.1:6379)

telemetry upgraded from capability provider

HTTP ingress ready at http://127.0.0.1:8080
demo start running (bundle=demo-bundle targets=[demo]); press Ctrl+C to stop
```

Key lines to highlight:
1. **`state backend: connected to Redis`** — state capability auto-discovered from gtpack
2. **`telemetry upgraded from capability provider`** — telemetry auto-configured from gtpack
3. **`HTTP ingress ready`** — gateway ready for webhooks + WebChat

---

## Step 5: Demo Messaging

### 5.1 WebChat (Zero Credentials)

1. Open browser: `http://localhost:8080/webchat`
2. Type a message → see echo response
3. Conversation state stored in Redis

### 5.2 Telegram (Live)

```bash
# In another terminal
GREENTIC_ENV=dev gtc op demo send \
  --bundle demo-bundle \
  --provider messaging-telegram \
  --to 7951102355 \
  --text "Hello from Greentic demo"
```

### 5.3 Slack (Live)

```bash
GREENTIC_ENV=dev gtc op demo send \
  --bundle demo-bundle \
  --provider messaging-slack \
  --to C0AFWP5C067 \
  --text "Hello from Greentic demo"
```

### 5.4 Teams (Live)

```bash
GREENTIC_ENV=dev gtc op demo send \
  --bundle demo-bundle \
  --provider messaging-teams \
  --to "c3392cbc-2cb0-48e8-9247-504d8defea40:19:wQzzrth6t3YA-aEdLzt8Pse3kW3Us-nJl9XzN-5NcEE1@thread.tacv2" \
  --text "Hello from Greentic demo"
```

---

## Step 6: Demo Telemetry

### 6.1 Stdout Mode

Telemetry spans appear in the operator terminal when messages are sent:

```
greentic.op{op.name="send_payload" provider.type="messaging.telegram"}
  greentic.op.requested{op.id=...}
  greentic.op.completed{op.id=... status="ok"}
```

### 6.2 Jaeger UI

1. Open http://localhost:16686
2. Select service: **greentic-operator**
3. Click **Find Traces**
4. Click a trace → see operation spans, durations, attributes

### 6.3 Talking Points

- "Telemetry is not hardcoded — it comes from a WASM capability provider"
- "Drop a different gtpack to switch from Jaeger to Honeycomb/Datadog"
- "Every operation emits structured spans: requested, completed, status"

---

## Step 7: Demo State Persistence

### 7.1 Show Redis Data

```bash
# See all Greentic keys
docker exec redis redis-cli KEYS "greentic*"

# Monitor in real-time
docker exec redis redis-cli MONITOR
```

### 7.2 Restart Persistence Test

1. Send a message via WebChat
2. `Ctrl+C` to stop operator
3. Restart operator: `GREENTIC_ENV=dev gtc op demo start --bundle demo-bundle`
4. Refresh WebChat → conversation preserved

### 7.3 Talking Points

- "State backend is auto-discovered from a gtpack — no config changes"
- "Redis gives persistence across restarts; swap to memory for dev"
- "Priority system: Redis (50) beats Memory (100)"

---

## Step 8: Demo Capability Auto-Discovery

### 8.1 Show It

```bash
# List all packs in bundle
ls demo-bundle/providers/messaging/*.gtpack

# Show capabilities
ls demo-bundle/state/runtime/demo/default/capabilities/
```

### 8.2 Talking Points

- "The platform discovers capabilities by scanning gtpack manifests"
- "No code changes needed — drop a gtpack, restart, it works"
- "Same pattern for telemetry, state, future capabilities (secrets, auth, etc.)"

---

## Cleanup

```bash
# Stop operator (Ctrl+C)

# Stop infrastructure
docker stop redis jaeger 2>/dev/null
docker rm redis jaeger 2>/dev/null
```

---

## Quick Reference

### Commands

```bash
# Start demo
GREENTIC_ENV=dev gtc op demo start --bundle demo-bundle

# Start with OTLP
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317 \
GREENTIC_ENV=dev gtc op demo start --bundle demo-bundle

# Send message
GREENTIC_ENV=dev gtc op demo send \
  --bundle demo-bundle --provider <provider> --to <target> --text "<message>"

# Check setup
GREENTIC_ENV=dev gtc op demo setup --bundle demo-bundle --best-effort

# List packs
GREENTIC_ENV=dev gtc op demo list-packs --bundle demo-bundle
```

### URLs

| Service | URL |
|---------|-----|
| WebChat | http://localhost:8080/webchat |
| HTTP Gateway | http://localhost:8080 |
| Jaeger UI | http://localhost:16686 |
| Public Webhook | https://xxxx.trycloudflare.com (from log) |

### Packs Summary

| Pack | Type | Capability |
|------|------|------------|
| `messaging-telegram.gtpack` | Messaging | Telegram bot |
| `messaging-slack.gtpack` | Messaging | Slack bot |
| `messaging-teams.gtpack` | Messaging | Teams bot |
| `messaging-webchat.gtpack` | Messaging | WebChat (Direct Line) |
| `messaging-webex.gtpack` | Messaging | Webex bot |
| `messaging-email.gtpack` | Messaging | Email (Graph API) |
| `messaging-whatsapp.gtpack` | Messaging | WhatsApp Cloud API |
| `messaging-dummy.gtpack` | Messaging | Dummy (testing) |
| `telemetry-otlp.gtpack` | Capability | OpenTelemetry pipeline |
| `state-redis.gtpack` | Capability | Redis KV store |
| `state-memory.gtpack` | Capability | In-memory KV store |
