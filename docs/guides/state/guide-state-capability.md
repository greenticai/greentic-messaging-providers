# State Capability (`greentic.cap.state.kv.v1`)

## Overview

The state capability provides a key-value store for flow nodes to persist working memory during execution. State is scoped per namespace (typically `{env}::{tenant}::{prefix}::{key}`) and supports TTL-based expiration.

## Capability ID

```
greentic.cap.state.kv.v1
```

## Operations

| Operation | Description | Payload Fields |
|-----------|-------------|----------------|
| `state.get` | Retrieve a value by key | `namespace`, `key` |
| `state.put` | Store a value | `namespace`, `key`, `value`, `ttl_seconds` (optional) |
| `state.delete` | Remove a key | `namespace`, `key` |
| `state.list` | List keys in a namespace | `namespace`, `key` (prefix filter) |
| `state.cas` | Compare-and-swap (conditional update) | `namespace`, `key`, `value`, `cas_version` |

## Payload Format

### StateOpPayload

```json
{
  "namespace": "dev::default::myapp::session",
  "key": "user_123_prefs",
  "value": [98, 121, 116, 101, 115],
  "ttl_seconds": 3600,
  "cas_version": null
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `namespace` | `String` | Yes | Scoped namespace for key isolation |
| `key` | `String` | Yes | Key within the namespace |
| `value` | `Vec<u8>` | For `put`/`cas` | Binary value (typically JSON-encoded) |
| `ttl_seconds` | `u32` | No | Time-to-live in seconds (0 = no expiry) |
| `cas_version` | `u64` | For `cas` | Expected version for compare-and-swap |

### StateOpResult

```json
{
  "value": [98, 121, 116, 101, 115],
  "found": true,
  "version": 42
}
```

| Field | Type | Description |
|-------|------|-------------|
| `value` | `Option<Vec<u8>>` | Retrieved value (None if not found) |
| `found` | `bool` | Whether the key exists |
| `version` | `Option<u64>` | Version/etag for CAS operations |

## Key Format

The canonical key format is:

```
{env}::{tenant}::{prefix}::{key}
```

- **env**: Environment identifier (e.g., `dev`, `staging`, `prod`)
- **tenant**: Tenant identifier for multi-tenant isolation
- **prefix**: Application or component prefix
- **key**: The actual key name

## Namespace Isolation

Each state operation is scoped to a namespace. The namespace ensures that:

1. Different tenants cannot access each other's state
2. Different environments are isolated
3. Application prefixes prevent key collisions between components

## Architecture: Native Dispatch

State operations are dispatched **natively** by the operator, not through WASM. This design choice is motivated by:

- **Performance**: State ops (get/put/delete) are on the hot path — every flow node may call them
- **Existing implementations**: `InMemoryStateStore` and `RedisStateStore` already work via the `StateStore` trait
- **Minimal overhead**: No WASM round-trip, no CBOR serialization per operation

The WASM component in state provider packs handles only:
- `describe` — component descriptor
- `qa-spec` / `apply-answers` — setup wizard
- `i18n-keys` / `i18n-bundle` — localization

## Provider Packs

Two state provider packs are available:

| Pack | Backend | Priority | Setup Required | Persistence |
|------|---------|----------|----------------|-------------|
| `state-memory` | In-memory | 100 (fallback) | No | Ephemeral |
| `state-redis` | Redis | 50 (preferred) | Yes | Persistent |

## Pack Manifest Example

```yaml
extensions:
  greentic.ext.capabilities.v1:
    kind: greentic.ext.capabilities.v1
    version: 1.0.0
    inline:
      schema_version: 1
      offers:
        - offer_id: state.memory.kv.01
          cap_id: greentic.cap.state.kv.v1
          version: v1
          provider:
            component_ref: state-provider-memory
            op: state.dispatch
          priority: 100
          requires_setup: false
          metadata:
            ephemeral: true
            backend: memory
```

## Hook Integration

State operations flow through the operator's hook chain:

1. **Pre-hook**: Can deny or modify the operation
2. **Native dispatch**: Actual KV operation on the backend
3. **Post-hook**: Can observe or log the result

Hook authors can target state operations via `applies_to.op_names`:

```yaml
applies_to:
  op_names:
    - state.put
    - state.delete
```

## See Also

- [Choosing a State Provider](guide-state-choosing-provider.md)
- [In-Memory State Setup](guide-state-setup-memory.md)
- [Redis State Setup](guide-state-setup-redis.md)
- [State Hooks and Policies](guide-state-hooks-policies.md)
