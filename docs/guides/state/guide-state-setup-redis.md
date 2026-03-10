# Redis State Provider Setup

## Overview

The Redis state provider (`state-redis`) provides a persistent key-value store backed by Redis. It supports TLS, connection pooling, and TTL-based key expiration.

## Prerequisites

- A running Redis server (v6.0+)
- Network access from the operator/runner to the Redis server
- (Optional) Redis credentials for authentication

## Installation

Include `state-redis.gtpack` in your deployment bundle:

```
deploy-bundle/
└── providers/
    └── state-redis.gtpack
```

## Configuration

The Redis provider requires setup. During `demo start` or pack provisioning, the QA wizard will prompt for connection details.

### Required Parameters

| Parameter | Type | Secret | Description |
|-----------|------|--------|-------------|
| `redis_url` | String | Yes | Redis connection URL (e.g., `redis://localhost:6379/0`) |

### Optional Parameters

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `redis_password` | String | Yes (redacted) | Redis authentication password |
| `redis_tls_enabled` | Boolean | false | Enable TLS for connections |
| `key_prefix` | String | `greentic` | Prefix for all keys in Redis |
| `default_ttl_seconds` | Integer | 0 | Default TTL (0 = no expiry) |
| `connection_pool_size` | Integer | 5 | Number of pooled connections |

### Connection URL Format

```
redis://[username:password@]host[:port][/database]
rediss://[username:password@]host[:port][/database]  # TLS
```

Examples:
- `redis://localhost:6379/0` — local Redis, database 0
- `redis://redis.example.com:6379/1` — remote Redis, database 1
- `rediss://redis.example.com:6380/0` — TLS connection

## Secrets

Redis credentials are stored in the Greentic secrets store:

```
secrets://dev/default/_/state-redis/redis_url
secrets://dev/default/_/state-redis/redis_password
```

### Seeding Secrets

```bash
# Using the seed-secret tool
greentic-operator seed-secret \
  --env dev --tenant default --team _ --category state-redis \
  --key redis_url --value "redis://localhost:6379/0"
```

## Pack Manifest

```yaml
pack_id: state-redis
version: 0.4.0
kind: capability

extensions:
  greentic.ext.capabilities.v1:
    inline:
      schema_version: 1
      offers:
        - offer_id: state.redis.kv.01
          cap_id: greentic.cap.state.kv.v1
          version: v1
          provider:
            component_ref: state-provider-redis
            op: state.dispatch
          priority: 50
          requires_setup: true
          setup:
            qa_ref: qa/state-redis-setup.json
          metadata:
            ephemeral: false
            backend: redis
```

## Key Mapping

Redis keys follow the format:

```
{key_prefix}:{namespace}:{key}
```

With the default prefix `greentic`, a key `user_123_prefs` in namespace `dev::default::myapp` becomes:

```
greentic:dev::default::myapp:user_123_prefs
```

## TLS Configuration

When `redis_tls_enabled` is `true`:

1. Use `rediss://` URL scheme (note the double `s`)
2. The client validates the server certificate against system CA roots
3. For self-signed certificates, add them to the system trust store

## Troubleshooting

### Connection refused

```
Error: Connection refused (os error 111)
```

- Verify Redis is running: `redis-cli ping`
- Check the URL host and port
- Ensure firewall allows the connection

### Authentication failed

```
Error: NOAUTH Authentication required
```

- Set the `redis_password` secret
- Verify the password matches the Redis `requirepass` setting

### TLS handshake failed

```
Error: SSL handshake failed
```

- Ensure `redis_tls_enabled` is `true`
- Use `rediss://` URL scheme
- Verify the server certificate is valid
