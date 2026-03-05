# Platform Architecture

## Tech Stack

| Aspect | Detail |
|--------|--------|
| Language | Rust (edition 2024), MSRV 1.89-1.91 |
| Wasm Runtime | Wasmtime v41 (component-model) |
| Wasm Target | `wasm32-wasip2` (WASI Preview 2) |
| WIT Bindgen | `wit-bindgen` v0.52-0.53 |
| HTTP Server | Axum v0.8 |
| Messaging Bus | NATS / `async-nats` v0.46 |
| Serialization | serde + serde_json + serde_yaml_bw + ciborium (CBOR) |
| Crypto | ed25519-dalek, blake3, sha2, aes-gcm |
| Observability | OpenTelemetry OTLP |

## Core Concepts

- **Flow**: YAML graph (`.ygtc`) of WASM component nodes
- **Pack**: Signed `.gtpack` archive (flows + components + assets + SBOM)
- **Component**: WASM building block implementing WIT interface
- **Provider**: WASM component bridging external services (messaging, events)
- **Tenant**: Multi-tenant isolation via `TenantCtx` (tenant_id, env_id)
- **Session**: Pausable/resumable flow execution state

## Dependency Hierarchy

```
greentic-types ←────────────────── (foundation, zero internal deps)
    ↑
greentic-telemetry
greentic-interfaces ← greentic-types
greentic-config ← greentic-types
    ↑
greentic-session ← greentic-types
greentic-state ← greentic-types + greentic-interfaces
greentic-flow ← greentic-interfaces + greentic-types + greentic-distributor-client
    ↑
greentic-pack ← greentic-flow + greentic-interfaces-host + greentic-config
greentic-component ← greentic-interfaces* + greentic-types + greentic-pack
greentic-mcp ← greentic-interfaces* + greentic-types
    ↑
greentic-runner ← ALL above
greentic-messaging ← greentic-flow + greentic-pack + greentic-types + greentic-interfaces*
```

## Provider Lifecycle in Operator

```
Provider Spec (YAML)
    ↓ [greentic-messaging-packgen]
Provider Pack (.gtpack)
    ├── setup.yaml           (questions for interactive setup)
    ├── components/
    │   ├── questions.wasm   (emit/validate setup questions)
    │   ├── provision.wasm   (write config/secrets to state-store)
    │   ├── <provider>.wasm  (adapter: send/reply/ingest/render_plan/encode/send_payload)
    │   └── <ingress>.wasm   (webhook handler - only for custom ingress mode)
    ├── flows/
    │   ├── setup_default.ygtc
    │   ├── diagnostics.ygtc
    │   ├── requirements.ygtc
    │   ├── verify_webhooks.ygtc
    │   └── sync_subscriptions.ygtc
    └── pack.yaml + extensions
    ↓ [greentic-operator]
    demo new → demo setup → demo build → demo start → demo send
```

## Operator Demo Flow

```
demo new        → Scaffold empty bundle directory
demo setup      → Run setup_default flow per provider (secrets + config)
demo build      → Resolve manifests, validate packs, create bundle
demo start      → Launch embedded runner + HTTP ingress server
demo send       → 3-phase pipeline: render_plan → encode → send_payload
demo receive    → Watch incoming messages
demo ingress    → Synthetic webhook ingress test
```

## Provider Component Ops Contract

Every messaging provider component implements via `invoke(op, input_json)`:

| Op | Purpose | Required For |
|----|---------|-------------|
| `send` | Send a message to a destination | `demo send` (legacy path) |
| `reply` | Reply to a specific message | Threading |
| `ingest_http` | Parse incoming webhook → ChannelMessageEnvelope | Ingress |
| `render_plan` | Determine render tier and plan | `demo send` phase 1 |
| `encode` | Encode message into ProviderPayloadV1 | `demo send` phase 2 |
| `send_payload` | Actually send encoded payload via HTTP | `demo send` phase 3 |
| `describe` | Return provider manifest (ops, config schema) | Discovery |
| `validate_config` | Validate config JSON | Setup |
| `healthcheck` | Provider health status | Diagnostics |

## WIT Interface (Provider Core)

```wit
interface schema-core-api {
    describe: func() -> list<u8>;
    validate-config: func(config-json: list<u8>) -> validation-result;
    healthcheck: func() -> health-status;
    invoke: func(op: string, input-json: list<u8>) -> invoke-result;
}
```

## Ingress Models

| Mode | Description | Providers |
|------|-------------|-----------|
| `custom` | Dedicated ingress WASM component (separate from adapter) | Telegram, Slack, Teams, WhatsApp |
| `default` | Uses `ingest_http` op on the main adapter component | Webex, Webchat |
| `none` | No ingress (outbound only) | Dummy, Email |
