# State Capability Provider — Demo Guide

Step-by-step guide to demo the state KV capability with `greentic-operator`.

## Prerequisites

- `greentic-operator` v0.4.32+ installed
- `demo-bundle/` with messaging providers
- Docker (for Redis)

## Architecture

```
┌──────────────────────────────────────────────────────┐
│ gtc op demo start                         │
│                                                      │
│  1. load packs ──────── discover state-redis.gtpack  │
│  2. capability registry ── resolve cap.state.kv.v1   │
│  3. resolve_state_backend()                          │
│     ├── check stable_id/pack_id for "redis"          │
│     ├── → StateBackendKind::Redis                    │
│     └── connect to Redis (url from config)           │
│  4. DemoRunnerHost uses Redis for all state ops      │
│  5. WebChat conversations persist across restarts    │
└──────────────────────────────────────────────────────┘
```

## Quick Start

### Step 1: Start Redis

```bash
docker run -d --name redis -p 6379:6379 redis:latest
```

Verify: `docker exec redis redis-cli ping` → `PONG`

### Step 2: Seed Redis URL to Secrets

The operator reads the Redis URL from the secrets store, not from env vars.

```bash
cd /root/works/personal/greentic

cargo run --manifest-path tools/seed-secret/Cargo.toml -- \
  demo-bundle/.greentic/dev/.dev.secrets.env \
  "secrets://dev/default/_/state-redis/redis_url" "redis://127.0.0.1:6379"
```

Verify:

```bash
cargo run --manifest-path tools/seed-secret/Cargo.toml -- \
  read demo-bundle/.greentic/dev/.dev.secrets.env \
  "secrets://dev/default/_/state-redis/redis_url"
```

Expected output: `redis://127.0.0.1:6379`

### Step 3: Build State Packs (if not already built)

```bash
cd /root/works/personal/greentic

# Build the generic capability-pack builder (one time)
cd tools/build-capability-pack && cargo build && cd ../..

# Build state-redis gtpack
./tools/build-capability-pack/target/debug/build-capability-pack \
  greentic-messaging-providers/packs/state-redis \
  demo-bundle/providers/messaging/state-redis.gtpack

# (Optional) Build state-memory gtpack
./tools/build-capability-pack/target/debug/build-capability-pack \
  greentic-messaging-providers/packs/state-memory \
  demo-bundle/providers/messaging/state-memory.gtpack
```

### Step 4: Create Install Record

```bash
mkdir -p demo-bundle/state/runtime/demo/default/capabilities

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

### Step 5: Start Operator

```bash
GREENTIC_ENV=dev gtc op demo start --bundle demo-bundle
```

To also enable OTLP telemetry (traces in Jaeger):

```bash
# Start Jaeger first
docker run -d --name jaeger \
  -p 4317:4317 -p 4318:4318 -p 16686:16686 \
  jaegertracing/all-in-one:latest

# Start operator with Redis + OTLP
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317 \
GREENTIC_ENV=dev gtc op demo start --bundle demo-bundle
```

### Step 6: Verify

Check the operator log for:

```
state.capability: offer=state.redis.kv.01 pack=state-redis priority=50
state backend: connected to Redis (url=redis://127.0.0.1:6379)
```

---

## Demo Scenarios

### Demo 1: State Persistence Across Restarts

Shows that state survives operator restart (unlike in-memory).

```bash
# Terminal 1: Ensure Redis is running
docker run -d --name redis -p 6379:6379 redis:7-alpine

# Terminal 1: Seed Redis URL (one time)
cargo run --manifest-path tools/seed-secret/Cargo.toml -- \
  demo-bundle/.greentic/dev/.dev.secrets.env \
  "secrets://dev/default/_/state-redis/redis_url" "redis://127.0.0.1:6379"

# Terminal 1: Start operator
GREENTIC_ENV=dev gtc op demo start --bundle demo-bundle

# Terminal 2: Send message via WebChat (creates conversation state)
# Open http://localhost:8080/webchat in browser
# Send a message — conversation is stored in Redis

# Terminal 1: Ctrl+C to stop operator
# Terminal 1: Restart operator
GREENTIC_ENV=dev gtc op demo start --bundle demo-bundle

# Terminal 2: Refresh WebChat — conversation history is preserved!
```

### Demo 2: Redis vs Memory Priority

When both packs are deployed, priority determines which backend is used.

| Pack | Priority | Behavior |
|------|----------|----------|
| `state-redis` | 50 | Used when Redis is available (lower number = higher priority) |
| `state-memory` | 100 | Fallback when Redis is not deployed |

To demo memory fallback:
1. Remove `state-redis.gtpack` from `providers/messaging/`
2. Remove the Redis install record
3. Restart operator — log shows: `state backend: using in-memory store (ephemeral)`

### Demo 3: Redis Data Inspection

```bash
# See all Greentic keys
docker exec redis redis-cli KEYS "greentic*"

# Inspect a specific key
docker exec redis redis-cli GET "greentic:state:demo:default:webchat:conversation-123"

# Monitor Redis in real-time
docker exec redis redis-cli MONITOR
```

---

## State Provider Packs

### state-redis

| Field | Value |
|-------|-------|
| Pack ID | `state-redis` |
| Capability | `greentic.cap.state.kv.v1` |
| Offer ID | `state.redis.kv.01` |
| Priority | 50 |
| Operation | `state.dispatch` |
| Component | `state-provider-redis` |

**Setup Questions (QA):**

| Field | Kind | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `redis_url` | String (secret) | Yes | — | Redis connection URL (e.g., `redis://localhost:6379/0`) |
| `redis_password` | String (secret) | No | — | Redis password |
| `redis_tls_enabled` | Boolean | No | `false` | Enable TLS |
| `key_prefix` | String | No | `greentic` | Key prefix to avoid collisions |
| `default_ttl_seconds` | Integer | No | `0` | Default TTL (0 = no expiry) |
| `connection_pool_size` | Integer | No | `5` | Connection pool size (1-100) |

### state-memory

| Field | Value |
|-------|-------|
| Pack ID | `state-memory` |
| Capability | `greentic.cap.state.kv.v1` |
| Offer ID | `state.memory.kv.01` |
| Priority | 100 |
| Operation | `state.dispatch` |
| Component | `state-provider-memory` |

**Characteristics:**
- No external dependencies
- Ephemeral — data lost on restart
- Good for dev/testing
- No setup questions required

---

## Bundle Structure

```
demo-bundle/
├── providers/
│   └── messaging/
│       ├── state-redis.gtpack        ← Redis state capability
│       ├── state-memory.gtpack       ← Memory state fallback
│       ├── telemetry-otlp.gtpack     ← Telemetry capability
│       ├── messaging-telegram.gtpack
│       └── ...
├── state/
│   └── runtime/
│       └── demo/
│           └── default/
│               └── capabilities/
│                   ├── state.redis.kv.01.install.json
│                   └── telemetry-otlp-v1.install.json
└── .greentic/
    └── dev/
        └── .dev.secrets.env
```

---

## Building the Pack Tool

The generic `build-capability-pack` tool reads `pack.yaml` and builds any capability gtpack:

```bash
# Build any capability pack
build-capability-pack <pack-source-dir> <output-gtpack-path>

# Examples
build-capability-pack packs/state-redis demo-bundle/providers/messaging/state-redis.gtpack
build-capability-pack packs/state-memory demo-bundle/providers/messaging/state-memory.gtpack
build-capability-pack packs/telemetry-otlp demo-bundle/providers/messaging/telemetry-otlp.gtpack
```

The tool:
1. Reads `pack.yaml` for pack metadata, components, flows, extensions
2. Encodes `manifest.cbor` via `greentic_types::cbor::encode_pack_manifest()`
3. Packages WASM, setup.yaml, QA, i18n, flows into a zip (gtpack)
4. Auto-generates `provider-extension.v1` from capability offers

---

## Troubleshooting

### "state backend: using in-memory store (ephemeral)" when Redis expected

1. Check `state-redis.gtpack` exists in `providers/messaging/`
2. Check install record exists at `state/runtime/demo/default/capabilities/state.redis.kv.01.install.json`
3. Ensure install record has `"status": "ready"`

### "state backend: using in-memory store" even after Redis seeded

1. Verify the secret was seeded correctly:
   ```bash
   cargo run --manifest-path tools/seed-secret/Cargo.toml -- \
     read demo-bundle/.greentic/dev/.dev.secrets.env \
     "secrets://dev/default/_/state-redis/redis_url"
   ```
2. Ensure `GREENTIC_ENV=dev` is set (secrets backend requires dev/test env)
3. Check that `state-redis.gtpack` and install record both exist (see Steps 3-4)

### Redis connection refused

```bash
# Check Redis is running
docker ps | grep redis

# Test connection
docker exec redis redis-cli ping

# Check port
netstat -tlnp | grep 6379
```

### Multiple state offers warning

```
state.capability.validation: multiple state KV capability offers found (2)
```

This is normal when both `state-redis` and `state-memory` are deployed. The highest-priority offer (lowest number) wins. Redis (priority=50) beats Memory (priority=100).

### Duplicate offers (4 instead of 2)

If you see 4 offers, the pack may be loaded twice. Check for duplicate gtpacks or install records.
