# Telemetry Setup Guide

This guide covers configuring OpenTelemetry (OTel) for the Greentic platform.

## Quick Start (stdout dev)

By default, greentic-operator uses `tracing_subscriber` with pretty-formatted stdout output. No configuration needed:

```bash
gtc op demo start
```

## Environment-Variable Configuration

Set `OTEL_EXPORTER_OTLP_ENDPOINT` to enable OTLP export:

```bash
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317 gtc op demo start
```

Additional env vars:

| Variable | Description | Default |
|----------|-------------|---------|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | OTLP collector endpoint | (none — stdout) |
| `TELEMETRY_EXPORT` | Export mode: `otlp-grpc`, `otlp-http`, `json-stdout` | `json-stdout` |
| `OTLP_HEADERS` | Comma-separated `key=value` headers | (none) |
| `TELEMETRY_SAMPLING` | Sampling strategy (see below) | `parent` |
| `OTLP_COMPRESSION` | Compression: `gzip` | (none) |

### Sampling Strategies

- `parent` — follow parent span decision
- `always_on` — sample everything
- `always_off` — sample nothing
- `traceidratio:0.5` — sample 50% of traces

## OTLP to Local Grafana/Jaeger

```bash
# Start Jaeger with OTLP receiver
docker run -d --name jaeger \
  -p 16686:16686 \
  -p 4317:4317 \
  jaegertracing/all-in-one:latest

# Start operator with OTLP
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317 \
  gtc op demo start

# View traces at http://localhost:16686
```

## Honeycomb Preset

```bash
TELEMETRY_PRESET=honeycomb \
HONEYCOMB_API_KEY=your-api-key \
  gtc op demo start
```

## Presets

| Preset | Endpoint | Required Env Var |
|--------|----------|-----------------|
| `honeycomb` | `https://api.honeycomb.io:443` | `HONEYCOMB_API_KEY` |
| `newrelic` | `https://otlp.nr-data.net:4317` | `NEW_RELIC_API_KEY` |
| `datadog` | `https://trace.agent.datadoghq.com:443` | `DD_API_KEY` |
| `elastic` | User-provided | `ELASTIC_APM_SECRET_TOKEN` |
| `grafana-tempo` | `http://localhost:4317` | (none) |
| `jaeger` | `http://localhost:4317` | (none) |
| `aws` | Auto-detected | AWS credentials |
| `gcp` | Auto-detected | GCP credentials |
| `azure` | Auto-detected | Azure credentials |

## Pack-Based Configuration (telemetry-otlp.gtpack)

For production deployments, use the telemetry provider pack:

1. Deploy `telemetry-otlp.gtpack` in your bundle's providers directory
2. Run QA setup wizard to configure endpoint, API key, preset
3. The operator auto-detects the telemetry capability and reconfigures the OTel pipeline

### Pack Setup Questions

| Field | Description |
|-------|-------------|
| `preset` | Backend preset (honeycomb, datadog, etc.) |
| `otlp_endpoint` | Collector endpoint URL |
| `otlp_api_key` | API key (stored as secret) |
| `export_mode` | otlp-grpc, otlp-http, json-stdout |
| `sampling_ratio` | 0.0 to 1.0 |
| `enable_operation_subs` | Enable operation subscription telemetry |
| `include_denied_ops` | Include denied operations in telemetry |

## Resource Attributes

Additional OTel resource attributes can be set via `TelemetryProviderConfig.resource_attributes`:

```json
{
  "resource_attributes": {
    "deployment.environment": "staging",
    "service.version": "1.2.3",
    "k8s.pod.name": "my-pod"
  }
}
```

## PII Redaction

Set `PII_MASK_REGEXES` or configure `redaction_patterns` in the provider config to mask sensitive data in spans:

```bash
PII_MASK_REGEXES="\\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\\.[A-Z|a-z]{2,}\\b,\\b\\d{3}-\\d{2}-\\d{4}\\b"
```
