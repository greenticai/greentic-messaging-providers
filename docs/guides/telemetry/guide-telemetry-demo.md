# Telemetry Capability Provider — Demo Guide

Step-by-step guide to demo the telemetry capability provider with `greentic-operator`.

## Prerequisites

- `greentic-operator` v0.4.32+ installed (`greentic-operator --version`)
- `demo-bundle/` directory with at least one messaging provider (e.g., Telegram, WebChat)
- `telemetry-otlp.gtpack` deployed in the bundle

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│ gtc op demo start                            │
│                                                         │
│  1. init_telemetry() ──────── env-var fallback (stdout) │
│  2. load packs ────────────── discover telemetry-otlp   │
│  3. capability registry ───── resolve cap.telemetry.v1  │
│  4. try_upgrade_telemetry()                             │
│     ├── invoke WASM component (telemetry.configure)     │
│     ├── receive TelemetryProviderConfig                 │
│     ├── init_from_provider_config() → OTel SDK          │
│     └── set_operation_subs_config() → structured spans  │
│  5. HTTP gateway ready                                  │
│  6. every operation emits greentic.op spans             │
└─────────────────────────────────────────────────────────┘
```

---

## Demo 1: Stdout Mode (Zero Setup)

The simplest demo — telemetry spans printed to terminal.

### Run

```bash
GREENTIC_ENV=dev gtc op demo start --bundle demo-bundle
```

### Expected Log

```
secrets runner ctx: ... provider_id=telemetry.configurator pack_id=telemetry-otlp ...
HTTP ingress ready at http://127.0.0.1:8080
demo start running ...
```

The telemetry capability is automatically discovered and invoked. Default config:
- `export_mode: "json-stdout"` — spans go to stdout
- `enable_operation_subs: true` — structured operation spans enabled
- `sampling_ratio: 1.0` — all traces captured

### Send a Test Message

```bash
# In another terminal
GREENTIC_ENV=dev gtc op demo send \
  --bundle demo-bundle \
  --provider messaging-telegram \
  --to 7951102355 \
  --message "Hello from telemetry demo"
```

Observe in the operator terminal — structured spans like:
```
greentic.op{op.name="send_payload" provider.type="messaging.telegram"}
  greentic.op.requested{op.id=... payload.size_bytes=...}
  greentic.op.completed{op.id=... status="ok" result.size_bytes=...}
```

---

## Demo 2: Jaeger (Visual Traces)

Full visual trace UI — see spans, durations, and relationships.

### Step 1: Start Jaeger

```bash
docker run -d --name jaeger \
  -p 16686:16686 \
  -p 4317:4317 \
  jaegertracing/all-in-one:latest
```

Verify: open http://localhost:16686 in browser.

### Step 2: Start Operator with OTLP

```bash
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317 \
GREENTIC_ENV=dev gtc op demo start --bundle demo-bundle
```

### Step 3: Send Messages

```bash
# Telegram
GREENTIC_ENV=dev gtc op demo send \
  --bundle demo-bundle \
  --provider messaging-telegram \
  --to 7951102355 \
  --text "Jaeger trace test"

# Slack
GREENTIC_ENV=dev gtc op demo send \
  --bundle demo-bundle \
  --provider messaging-slack \
  --to C0AFWP5C067 \
  --text "Jaeger trace test"
```

### Step 4: View in Jaeger

1. Open http://localhost:16686
2. Select service: **greentic-operator**
3. Click **Find Traces**
4. Click a trace to see:
   - `greentic.op` root span (provider invocation)
   - `greentic.op.requested` event (before call)
   - `greentic.op.completed` event (after call, with status)
   - Duration, attributes, hierarchy

### Cleanup

```bash
docker stop jaeger && docker rm jaeger
```

---

## Demo 3: Grafana + Tempo Stack

Production-like setup with Grafana dashboard.

### Step 1: Create docker-compose.yaml

```yaml
version: "3"
services:
  tempo:
    image: grafana/tempo:latest
    command: ["-config.file=/etc/tempo.yaml"]
    volumes:
      - ./tempo.yaml:/etc/tempo.yaml
    ports:
      - "4317:4317"   # OTLP gRPC
      - "3200:3200"   # Tempo query

  grafana:
    image: grafana/grafana:latest
    environment:
      - GF_AUTH_ANONYMOUS_ENABLED=true
      - GF_AUTH_ANONYMOUS_ORG_ROLE=Admin
    volumes:
      - ./grafana-datasources.yaml:/etc/grafana/provisioning/datasources/ds.yaml
    ports:
      - "3000:3000"
```

### Step 2: Create tempo.yaml

```yaml
server:
  http_listen_port: 3200
distributor:
  receivers:
    otlp:
      protocols:
        grpc:
          endpoint: "0.0.0.0:4317"
storage:
  trace:
    backend: local
    local:
      path: /tmp/tempo/blocks
```

### Step 3: Create grafana-datasources.yaml

```yaml
apiVersion: 1
datasources:
  - name: Tempo
    type: tempo
    access: proxy
    url: http://tempo:3200
    isDefault: true
```

### Step 4: Start

```bash
docker compose up -d

OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317 \
GREENTIC_ENV=dev gtc op demo start --bundle demo-bundle
```

### Step 5: View

1. Open http://localhost:3000 (Grafana)
2. Go to Explore → Tempo datasource
3. Search for traces by service name `greentic-operator`

---

## Demo 4: Honeycomb (Cloud SaaS)

### Step 1: Get API Key

1. Sign up at https://www.honeycomb.io (free tier available)
2. Go to Account → Team Settings → API Keys
3. Copy the API key

### Step 2: Seed Secret

```bash
cd demo-bundle
# Add to secrets store
GREENTIC_ENV=dev gtc op demo capability seed-secret \
  --bundle . \
  --provider telemetry-otlp \
  --key otlp_api_key \
  --value "YOUR_HONEYCOMB_API_KEY"
```

Or set via environment:

```bash
TELEMETRY_PRESET=honeycomb \
HONEYCOMB_API_KEY=your-api-key \
OTEL_EXPORTER_OTLP_ENDPOINT=https://api.honeycomb.io:443 \
GREENTIC_ENV=dev gtc op demo start --bundle demo-bundle
```

### Step 3: View

Go to https://ui.honeycomb.io → your dataset → see traces with full operation spans.

---

## All Supported OTLP Backends

The telemetry pack supports **10 backend presets** in a single gtpack. The preset determines the default endpoint, auth header, and export mode.

### How Presets Work

```
telemetry-otlp.gtpack (1 pack)
  └── telemetry.configure operation
        ├── reads "preset" from state/config
        ├── reads "otlp_api_key" from secrets
        └── returns TelemetryProviderConfig
              └── greentic-telemetry resolves preset
                    ├── honeycomb → endpoint + x-honeycomb-team header
                    ├── datadog → endpoint + DD_API_KEY header
                    ├── jaeger → endpoint, no auth
                    └── ... etc
```

You do NOT need separate packs per backend. One `telemetry-otlp.gtpack` handles all of them.

### Quick Reference

| Preset | Default Endpoint | Auth Header | Auth Env Var | Free Tier |
|--------|------------------|-------------|--------------|-----------|
| `jaeger` | `http://localhost:4317` | — | — | Self-hosted |
| `grafana-tempo` | `http://localhost:4317` | via `OTLP_HEADERS` | — | Free (local) / Grafana Cloud |
| `honeycomb` | `https://api.honeycomb.io:443` | `x-honeycomb-team` | `HONEYCOMB_API_KEY` | 20M events/mo |
| `datadog` | `http://datadog-agent:4317` | `DD_API_KEY` | `DD_API_KEY` | 14-day trial |
| `newrelic` | `https://otlp.nr-data.net:4317` | `api-key` | `NEW_RELIC_API_KEY` | 100GB/mo |
| `elastic` | (user-provided) | `Authorization: Bearer` | `ELASTIC_APM_SECRET_TOKEN` | Free (local) |
| `aws` | `http://aws-otel-collector:4317` | AWS IAM | AWS creds | Pay-per-use |
| `gcp` | `http://otc-collector:4317` | GCP IAM | GCP creds | Pay-per-use |
| `azure` | `http://otel-collector-azure:4317` | Azure IAM | Azure creds | Pay-per-use |
| `loki` | (user-provided) | — | — | Self-hosted |

### Per-Backend Setup

---

#### Jaeger (Local, Recommended for Dev)

Best for: local development, quick visual debugging.

```bash
# Start
docker run -d --name jaeger \
  -p 16686:16686 \
  -p 4317:4317 \
  jaegertracing/all-in-one:latest

# Run operator
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317 \
GREENTIC_ENV=dev gtc op demo start --bundle demo-bundle

# View: http://localhost:16686 → service "greentic-operator"
```

---

#### Grafana Tempo (Local or Cloud)

Best for: production-like setup, Grafana dashboard integration.

**Local:**
```bash
# See "Demo 3" section above for full docker-compose setup

OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317 \
GREENTIC_ENV=dev gtc op demo start --bundle demo-bundle

# View: http://localhost:3000 (Grafana) → Explore → Tempo
```

**Grafana Cloud:**
```bash
OTEL_EXPORTER_OTLP_ENDPOINT=https://tempo-us-central1.grafana.net:443 \
OTLP_HEADERS="Authorization=Basic $(echo -n 'instance_id:api_key' | base64)" \
GREENTIC_ENV=dev gtc op demo start --bundle demo-bundle
```

---

#### Honeycomb

Best for: cloud-native SaaS observability, rich query UI.

```bash
# Sign up: https://www.honeycomb.io (20M events/mo free)
# Get API key: Account → Team Settings → API Keys

OTEL_EXPORTER_OTLP_ENDPOINT=https://api.honeycomb.io:443 \
HONEYCOMB_API_KEY=your-api-key \
GREENTIC_ENV=dev gtc op demo start --bundle demo-bundle

# View: https://ui.honeycomb.io → select dataset → query traces
```

---

#### Datadog

Best for: existing Datadog users, APM + infrastructure correlation.

```bash
# Requires Datadog Agent with OTLP receiver enabled
# In datadog.yaml: otlp_config.receiver.protocols.grpc.endpoint: "0.0.0.0:4317"

docker run -d --name dd-agent \
  -e DD_API_KEY=your-dd-api-key \
  -e DD_OTLP_CONFIG_RECEIVER_PROTOCOLS_GRPC_ENDPOINT=0.0.0.0:4317 \
  -p 4317:4317 \
  gcr.io/datadoghq/agent:latest

OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317 \
DD_API_KEY=your-dd-api-key \
GREENTIC_ENV=dev gtc op demo start --bundle demo-bundle

# View: https://app.datadoghq.com → APM → Traces
```

---

#### New Relic

Best for: full-stack observability, generous free tier (100GB/mo).

```bash
# Sign up: https://newrelic.com (100GB/mo free)
# Get API key: API Keys → INGEST - LICENSE

OTEL_EXPORTER_OTLP_ENDPOINT=https://otlp.nr-data.net:4317 \
NEW_RELIC_API_KEY=your-license-key \
GREENTIC_ENV=dev gtc op demo start --bundle demo-bundle

# View: https://one.newrelic.com → APM → Services → greentic-operator
```

---

#### Elastic APM

Best for: existing Elastic/Kibana users, log + trace correlation.

```bash
# Self-hosted Elastic APM Server or Elastic Cloud
# Get secret token from APM Server config

OTEL_EXPORTER_OTLP_ENDPOINT=https://your-apm-server:8200 \
ELASTIC_APM_SECRET_TOKEN=your-secret-token \
GREENTIC_ENV=dev gtc op demo start --bundle demo-bundle

# View: Kibana → Observability → APM → Services
```

---

#### AWS (X-Ray via ADOT Collector)

Best for: AWS-native, X-Ray integration.

```bash
# Deploy ADOT (AWS Distro for OpenTelemetry) collector
# https://aws-otel.github.io/docs/getting-started/collector

# ECS/EKS: sidecar ADOT container
# EC2: install ADOT collector binary

OTEL_EXPORTER_OTLP_ENDPOINT=http://aws-otel-collector:4317 \
GREENTIC_ENV=dev gtc op demo start --bundle demo-bundle

# View: AWS Console → CloudWatch → X-Ray traces
```

---

#### GCP (Cloud Trace via OTel Collector)

Best for: GCP-native, Cloud Trace integration.

```bash
# Deploy OTel Collector with GCP exporter
# https://cloud.google.com/trace/docs/setup/otlp

OTEL_EXPORTER_OTLP_ENDPOINT=http://otc-collector:4317 \
GREENTIC_ENV=dev gtc op demo start --bundle demo-bundle

# View: GCP Console → Trace → Trace list
```

---

#### Azure Monitor (via OTel Collector)

Best for: Azure-native, Application Insights integration.

```bash
# Deploy OTel Collector with Azure Monitor exporter
# https://learn.microsoft.com/en-us/azure/azure-monitor/app/opentelemetry-configuration

OTEL_EXPORTER_OTLP_ENDPOINT=http://otel-collector-azure:4317 \
GREENTIC_ENV=dev gtc op demo start --bundle demo-bundle

# View: Azure Portal → Application Insights → Transaction search
```

---

#### Loki (Logs Only)

Best for: log aggregation (not tracing). Uses stdout JSON mode.

```bash
# Loki collects logs, not traces — use json-stdout export mode
# Pipe operator stdout to Promtail/Alloy for Loki ingestion

GREENTIC_ENV=dev gtc op demo start --bundle demo-bundle \
  2>&1 | promtail --stdin --client.url=http://localhost:3100/loki/api/v1/push

# View: Grafana → Explore → Loki datasource
```

---

### Switching Between Backends

To switch backend, just change the env vars. No pack rebuild needed:

```bash
# Monday: dev with Jaeger
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317 \
  gtc op demo start --bundle demo-bundle

# Tuesday: staging with Honeycomb
OTEL_EXPORTER_OTLP_ENDPOINT=https://api.honeycomb.io:443 \
HONEYCOMB_API_KEY=xxx \
  gtc op demo start --bundle demo-bundle

# Wednesday: production with Datadog
OTEL_EXPORTER_OTLP_ENDPOINT=http://datadog-agent:4317 \
DD_API_KEY=xxx \
  gtc op demo start --bundle demo-bundle
```

---

## What the Demo Shows

### Capability Auto-Discovery

The operator automatically:
1. Scans all gtpacks in the bundle
2. Finds `greentic.cap.telemetry.v1` offer in telemetry-otlp manifest
3. Checks install record (`state/runtime/demo/default/capabilities/telemetry-otlp-v1.install.json`)
4. Invokes the WASM component to get telemetry config
5. Upgrades the OTel pipeline at runtime

This is the **capability provider pattern** — platform behavior is extended via gtpack plugins without modifying operator code.

### Operation Subscriptions

Every provider operation (send, ingress, render, etc.) emits structured spans:

| Span | When | Attributes |
|------|------|------------|
| `greentic.op` | Root span per operation | op.name, provider.type, tenant.id, team.id |
| `greentic.op.requested` | Before invocation | op.id, payload.size_bytes |
| `greentic.op.completed` | After invocation | op.id, status (ok/err/denied), result.size_bytes |

### Talking Points

- **Zero-config**: Just drop the gtpack in the bundle — telemetry works
- **Dynamic**: Telemetry config comes from a WASM component, not hardcoded
- **Multi-backend**: Supports Jaeger, Honeycomb, Datadog, Grafana Tempo, New Relic, Elastic, AWS, GCP, Azure
- **Secure**: API keys stored in secrets backend, never in config files
- **Observable**: Full operation lifecycle with structured spans and correlation

---

## Bundle Structure

```
demo-bundle/
├── greentic.demo.yaml
├── providers/
│   └── messaging/
│       ├── messaging-telegram.gtpack
│       ├── messaging-slack.gtpack
│       └── telemetry-otlp.gtpack          ← telemetry capability pack
├── state/
│   └── runtime/
│       └── demo/
│           └── default/
│               └── capabilities/
│                   └── telemetry-otlp-v1.install.json  ← install record
└── .greentic/
    └── dev/
        └── .dev.secrets.env                ← secrets (otlp_api_key if set)
```

---

## Rebuilding the Pack

If you modify the component source:

### Step 1: Build WASM

```bash
cd component-telemetry-provider
cargo build --target wasm32-wasip2 --release
```

Output: `target/wasm32-wasip2/release/component_telemetry_provider.wasm`

### Step 2: Copy WASM to pack source

```bash
cp target/wasm32-wasip2/release/component_telemetry_provider.wasm \
   ../packs/telemetry-otlp/components/component-telemetry-provider.wasm
```

### Step 3: Build gtpack

```bash
cd ../tools/build-telemetry-pack
cargo run
```

Output: `demo-bundle/providers/messaging/telemetry-otlp.gtpack`

### Step 4: Verify

```bash
zipinfo demo-bundle/providers/messaging/telemetry-otlp.gtpack
```

Expected contents:
```
manifest.cbor
components/component-telemetry-provider.wasm
setup.yaml
i18n/en.json
i18n/id.json
flows/setup_default/flow.ygtc
flows/setup_default/flow.json
flows/requirements/flow.ygtc
flows/requirements/flow.json
```

---

## Troubleshooting

### "no reactor running" panic

**Cause**: OTLP gRPC exporter needs Tokio runtime but operator's `main()` is synchronous.

**Fix**: Update `greentic-telemetry` to latest (v0.4.3+) which auto-creates a runtime. Or rebuild operator from source.

### "telemetry capability invocation returned failure outcome"

**Cause**: WASM component failed to execute.

**Check**: Look at the error detail in the log. Common causes:
- Missing `greentic.provider-extension.v1` in manifest
- WASM imports `state-store` or `secrets-store` not supported by linker

### "missing field config_state_keys" in install record

**Fix**: Ensure install record has all required fields:

```json
{
  "cap_id": "greentic.cap.telemetry.v1",
  "stable_id": "telemetry-otlp-v1",
  "pack_id": "telemetry-otlp",
  "status": "ready",
  "config_state_keys": [],
  "timestamp_unix_sec": 1741035600
}
```

### No traces appearing in Jaeger

1. Verify Jaeger is running: `docker ps | grep jaeger`
2. Verify OTLP port: `curl -s http://localhost:4317` (should not refuse connection)
3. Check operator log for `telemetry upgraded from capability provider`
4. Send a message to generate traces: `demo send --provider messaging-telegram ...`
5. Wait a few seconds (batch exporter has a flush interval)

### Stale WASM cache

If component changes are not reflected after rebuild:
```bash
rm -rf component-telemetry-provider/target/wasm32-wasip2/
cargo build --target wasm32-wasip2 --release
```

---

## TelemetryProviderConfig Reference

Full config returned by the telemetry.configure operation:

```json
{
  "export_mode": "json-stdout",
  "endpoint": null,
  "headers": {},
  "sampling_ratio": 1.0,
  "compression": null,
  "service_name": null,
  "resource_attributes": {},
  "redaction_patterns": [],
  "preset": null,
  "enable_operation_subs": true,
  "operation_subs_mode": null,
  "include_denied_ops": true,
  "payload_policy": null
}
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `export_mode` | string | `"json-stdout"` | `"otlp-grpc"`, `"otlp-http"`, `"json-stdout"`, `"none"` |
| `endpoint` | string? | null | OTLP collector URL (e.g., `http://localhost:4317`) |
| `headers` | object | `{}` | Auth/metadata headers |
| `sampling_ratio` | float | `1.0` | 0.0 (off) to 1.0 (all) |
| `compression` | string? | null | `"gzip"` or null |
| `service_name` | string? | null | OTel service name (defaults to `"greentic-operator"`) |
| `resource_attributes` | object | `{}` | Additional OTel resource attrs |
| `redaction_patterns` | string[] | `[]` | Regex patterns for PII masking |
| `preset` | string? | null | Backend preset name |
| `enable_operation_subs` | bool | `true` | Enable operation subscription spans |
| `operation_subs_mode` | string? | null | `"metrics_only"`, `"traces_only"`, `"metrics_and_traces"` |
| `include_denied_ops` | bool | `true` | Include denied ops in telemetry |
| `payload_policy` | string? | null | `"none"`, `"hash_only"` |

## Supported Presets

| Preset | Endpoint | Auth Header | Env Var |
|--------|----------|-------------|---------|
| `honeycomb` | `https://api.honeycomb.io:443` | `x-honeycomb-team` | `HONEYCOMB_API_KEY` |
| `datadog` | `http://datadog-agent:4317` | `DD_API_KEY` | `DD_API_KEY` |
| `newrelic` | `https://otlp.nr-data.net:4317` | `api-key` | `NEW_RELIC_API_KEY` |
| `elastic` | (user-provided) | `Authorization: Bearer` | `ELASTIC_APM_SECRET_TOKEN` |
| `grafana-tempo` | `http://localhost:4317` | (none) | — |
| `jaeger` | `http://localhost:4317` | (none) | — |
| `aws` | `http://aws-otel-collector:4317` | (none) | AWS creds |
| `gcp` | `http://otc-collector:4317` | (none) | GCP creds |
| `azure` | `http://otel-collector-azure:4317` | (none) | Azure creds |
| `loki` | (user-provided) | (none) | — |
