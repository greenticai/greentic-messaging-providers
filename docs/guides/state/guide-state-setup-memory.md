# In-Memory State Provider Setup

## Overview

The in-memory state provider (`state-memory`) provides an ephemeral key-value store that runs entirely in-process. No external services are required.

## Installation

Include `state-memory.gtpack` in your demo or deployment bundle:

```
demo-bundle/
└── providers/
    └── state-memory.gtpack
```

## Configuration

The in-memory provider works out of the box with no required configuration. Optional settings can be adjusted through the QA setup wizard.

### Optional Parameters

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `max_entries` | Integer | 10000 | Maximum number of entries in the store |
| `default_ttl_seconds` | Integer | 0 | Default TTL for entries (0 = no expiry) |

### Pack Manifest

```yaml
pack_id: state-memory
version: 0.4.0
kind: capability

extensions:
  greentic.ext.capabilities.v1:
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

## Behavior

- **Ephemeral**: All data is lost when the process restarts
- **No network**: All operations are in-process, sub-microsecond latency
- **Namespace isolation**: Keys are scoped by namespace string
- **Thread-safe**: Uses interior mutability for concurrent access

## Limitations

- Data does not survive process restarts
- Not suitable for multi-instance deployments
- Memory usage grows with number of stored entries
- No built-in replication or backup

## Typical Use

```bash
# Start operator with in-memory state (default)
gtc op demo start
```

The operator automatically selects the in-memory provider when no Redis provider is configured.
