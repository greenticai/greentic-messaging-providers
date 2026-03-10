# State Hooks and Policies

## Overview

State operations flow through the operator's hook chain, allowing policies to control access, audit operations, and enforce business rules on key-value state.

## Hook Chain Flow

```
Flow Node → state.put request
  → Pre-hook chain (can deny/modify)
  → Native dispatch (InMemory/Redis)
  → Post-hook chain (observe/log)
  → Result returned to flow node
```

## Targeting State Operations

Hook capabilities can target specific state operations using the `applies_to` configuration:

```yaml
# In a hook capability pack
extensions:
  greentic.ext.capabilities.v1:
    inline:
      schema_version: 1
      offers:
        - offer_id: my-hook.state-audit.01
          cap_id: greentic.cap.op_hook.pre
          version: v1
          provider:
            component_ref: my-hook-component
            op: hook.pre
          applies_to:
            op_names:
              - state.put
              - state.delete
```

### Available Operation Names

| Name | Triggered When |
|------|---------------|
| `state.get` | Reading a key |
| `state.put` | Writing a key |
| `state.delete` | Deleting a key |
| `state.list` | Listing keys |
| `state.cas` | Compare-and-swap |

## Operation Envelope

The hook receives an `OperationEnvelope` containing:

```json
{
  "op_name": "state.put",
  "payload": {
    "namespace": "dev::default::myapp::session",
    "key": "user_123_prefs",
    "ttl_seconds": 3600
  },
  "context": {
    "tenant_id": "default",
    "env_id": "dev",
    "flow_id": "main",
    "node_id": "save_prefs"
  }
}
```

**Note**: The `value` field is intentionally omitted from the hook envelope to prevent sensitive data from leaking through the hook chain. Only the key, namespace, and metadata are visible to hooks.

## Common Patterns

### Access Control

Deny writes to restricted namespaces:

```yaml
# Pre-hook that denies state.put to admin namespace
applies_to:
  op_names:
    - state.put
    - state.delete
```

The hook component checks the namespace and returns a deny result if the namespace starts with `admin::`.

### Audit Logging

Log all state mutations for compliance:

```yaml
# Post-hook that logs state changes
applies_to:
  op_names:
    - state.put
    - state.delete
    - state.cas
```

The hook component emits structured audit events with:
- Operation type
- Namespace and key (never the value)
- Timestamp
- Tenant and flow context
- Success/failure status

### Rate Limiting

Limit the frequency of state operations per tenant:

```yaml
applies_to:
  op_names:
    - state.put
    - state.get
```

The pre-hook tracks operation counts and denies requests exceeding the configured rate.

## Subscription Events

State operations emit subscription events that can be observed asynchronously:

### Pre-subscription

Emitted before the native dispatch:

```json
{
  "event": "state.op.pre",
  "op": "state.put",
  "namespace": "dev::default::myapp",
  "key": "user_123",
  "tenant": "default",
  "timestamp": "2026-03-03T10:00:00Z"
}
```

### Post-subscription

Emitted after the native dispatch completes:

```json
{
  "event": "state.op.post",
  "op": "state.put",
  "namespace": "dev::default::myapp",
  "key": "user_123",
  "success": true,
  "duration_ms": 2,
  "timestamp": "2026-03-03T10:00:00.002Z"
}
```

## Security Considerations

1. **Never log values**: State values may contain sensitive data. Only log key hashes for audit
2. **Namespace isolation**: Hooks should not allow cross-tenant namespace access
3. **Deny by default**: For sensitive namespaces, use pre-hooks to deny all operations and explicitly allow known callers
4. **Hook performance**: Pre-hooks add latency to every state operation. Keep hook logic minimal
