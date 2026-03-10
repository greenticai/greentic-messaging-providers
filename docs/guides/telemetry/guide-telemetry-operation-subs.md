# Telemetry: Operation Subscriptions

Operation subscriptions emit structured tracing spans for every operation in the Greentic pipeline. This enables observability of the full request lifecycle.

## What Gets Emitted

### Root Span

Every provider/capability invocation creates a root span:

```
greentic.op
├── greentic.op.name = "send_payload"
├── greentic.provider.type_ = "messaging.telegram"
├── greentic.tenant.id = "acme"
├── greentic.team.id = "default"
└── otel.status_code = "OK" | "ERROR"
```

### Operation Requested Event

Emitted before the provider is invoked:

```
greentic.op.requested
├── greentic.op.id = "op-abc123"
├── greentic.op.name = "send_payload"
├── greentic.tenant.id = "acme"
├── greentic.team.id = "default"
└── greentic.payload.size_bytes = 1024  (only with HashOnly policy)
```

### Operation Completed Event

Emitted after the provider returns:

```
greentic.op.completed
├── greentic.op.id = "op-abc123"
├── greentic.op.name = "send_payload"
├── greentic.op.status = "ok" | "err" | "denied"
├── greentic.tenant.id = "acme"
├── greentic.team.id = "default"
└── greentic.result.size_bytes = 512  (only with HashOnly policy)
```

## Configuration

### OperationSubsConfig

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | bool | `true` | Master switch for operation sub telemetry |
| `mode` | SubsMode | `MetricsAndTraces` | What to emit |
| `include_denied` | bool | `true` | Include denied operations |
| `payload_policy` | PayloadPolicy | `None` | Payload data inclusion |

### SubsMode

| Mode | Traces | Metrics |
|------|--------|---------|
| `MetricsAndTraces` | Yes | Yes |
| `TracesOnly` | Yes | No |
| `MetricsOnly` | No | Yes |

### PayloadPolicy

| Policy | Behavior |
|--------|----------|
| `None` | No payload size in spans (safe default) |
| `HashOnly` | Include `payload.size_bytes` / `result.size_bytes` |

## Enabling / Disabling

### Via Pack Config (setup.yaml)

```yaml
enable_operation_subs: true
operation_subs_mode: metrics_and_traces  # or: metrics_only, traces_only
include_denied_ops: true
payload_policy: none  # or: hash_only
```

### Via TelemetryProviderConfig

```json
{
  "enable_operation_subs": true,
  "operation_subs_mode": "metrics_and_traces",
  "include_denied_ops": true,
  "payload_policy": "none"
}
```

## Correlation

All sub-events (requested, completed, hook invocations) are children of the root `greentic.op` span. This enables trace correlation in backends like Jaeger, Honeycomb, and Grafana Tempo.

The root span's `otel.status_code` is set to `"OK"` on success or `"ERROR"` on failure, following OpenTelemetry conventions.

## Filtering Denied Operations

When `include_denied` is `false`, operations with status `"denied"` (blocked by pre-hooks) are silently dropped from completed events. Requested events are still emitted since denial happens after the request is logged.
