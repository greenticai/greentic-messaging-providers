# Telemetry Capability Specification

## Capability ID

```
greentic.cap.telemetry.v1
```

## Overview

The telemetry capability allows the OTel pipeline to be configured dynamically via a pack-deployed provider component. The operator resolves this capability at startup, invokes the provider, and uses the returned configuration to initialize (or upgrade) the telemetry pipeline.

## Config Model

The provider component returns a `TelemetryProviderConfig` JSON payload:

```json
{
  "export_mode": "otlp-grpc",
  "endpoint": "http://localhost:4317",
  "headers": { "x-honeycomb-team": "..." },
  "sampling_ratio": 1.0,
  "compression": "gzip",
  "service_name": "greentic-operator",
  "resource_attributes": {
    "deployment.environment": "staging"
  },
  "redaction_patterns": [],
  "preset": "honeycomb",
  "enable_operation_subs": true,
  "operation_subs_mode": "metrics_and_traces",
  "include_denied_ops": true,
  "payload_policy": "none"
}
```

### Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `export_mode` | string | No | `json-stdout` | `otlp-grpc`, `otlp-http`, `json-stdout`, `none` |
| `endpoint` | string? | No | Auto | OTLP collector endpoint |
| `headers` | map | No | `{}` | Auth/metadata headers |
| `sampling_ratio` | f64 | No | `1.0` | 0.0 (off) to 1.0 (all) |
| `compression` | string? | No | null | `gzip` or null |
| `service_name` | string? | No | `greentic-operator` | OTel service name |
| `resource_attributes` | map | No | `{}` | Additional OTel resource attributes |
| `redaction_patterns` | string[] | No | `[]` | Regex patterns for PII redaction |
| `preset` | string? | No | null | Backend preset name |
| `enable_operation_subs` | bool | No | `true` | Enable operation sub telemetry |
| `operation_subs_mode` | string? | No | null | `metrics_only`, `traces_only`, `metrics_and_traces` |
| `include_denied_ops` | bool | No | `true` | Include denied ops in telemetry |
| `payload_policy` | string? | No | null | `none` or `hash_only` |

## Pack Extension

Register the capability in `pack.yaml`:

```yaml
extensions:
  greentic.ext.capabilities.v1:
    kind: greentic.ext.capabilities.v1
    version: 0.4.0
    inline:
      schema_version: 1
      offers:
        - offer_id: "telemetry-otlp-v1"
          cap_id: "greentic.cap.telemetry.v1"
          version: "v1"
          provider:
            component_ref: "component-telemetry-provider"
            op: "telemetry.configure"
          priority: 100
          requires_setup: true
          setup:
            qa_ref: "setup.yaml"
```

## Provider Component Contract

The telemetry provider component must implement:

- **Op**: `telemetry.configure`
- **Input**: `{}` (empty JSON)
- **Output**: `TelemetryProviderConfig` JSON (see above)
- **Side effects**: Reads from `secrets-store` (endpoint, API key) and `state-store` (config values)

## Resolution Flow

```
1. Operator starts → init_telemetry() with env-var config (fallback)
2. DemoRunnerHost created → CapabilityRegistry built from pack manifests
3. try_upgrade_telemetry() called:
   a. Resolve CAP_TELEMETRY_V1 from registry
   b. If found → invoke_capability("telemetry.configure", "{}")
   c. Parse TelemetryProviderConfig from outcome.output
   d. Call init_from_provider_config() (idempotent OTel SDK init)
   e. Derive OperationSubsConfig → set on runner host
4. If no telemetry capability → env-var config remains active
```

## Validation Rules

The operator validates telemetry offers at startup:

1. **Multiple offers**: Warning if more than one telemetry offer exists (only highest-priority used)
2. **Non-standard op**: Warning if `provider_op` is not `telemetry.configure`
3. **Missing QA ref**: Warning if `requires_setup: true` but no `setup.qa_ref`

## Idempotency

`init_from_provider_config()` is idempotent — it checks a `OnceCell` guard and skips re-initialization if the OTel SDK is already configured. This means:

- First call (env-var based) configures the baseline
- Second call (capability-based) upgrades if the first hasn't run, or is a no-op if it has
- Multiple runner hosts can safely call it concurrently
