# Choosing a State Provider

## Overview

Greentic supports multiple state backends through the capability system. This guide helps you choose the right provider for your use case.

## Comparison

| Feature | In-Memory (`state-memory`) | Redis (`state-redis`) |
|---------|---------------------------|----------------------|
| **Persistence** | Ephemeral (lost on restart) | Persistent across restarts |
| **Setup** | None required | Redis URL + credentials |
| **Performance** | Fastest (in-process) | Fast (network round-trip) |
| **Scalability** | Single process only | Multi-process, distributed |
| **TTL Support** | Basic (in-process eviction) | Native Redis TTL |
| **Use Case** | Dev/test, demos, prototyping | Staging, production |
| **Priority** | 100 (fallback) | 50 (preferred) |
| **Data Isolation** | Process-scoped | Server-scoped |

## When to Use In-Memory

- **Local development** — quick iteration without external dependencies
- **Demo environments** — `gtc op demo start` uses memory by default
- **Unit/integration tests** — deterministic, fast, no cleanup needed
- **Stateless flows** — when flow state doesn't need to survive restarts

## When to Use Redis

- **Production deployments** — state survives process restarts and deployments
- **Multi-instance setups** — shared state across multiple operator/runner instances
- **Long-running sessions** — conversations or workflows that span hours/days
- **Audit requirements** — Redis provides durability guarantees

## Priority and Resolution

When both providers are available, the operator resolves the state backend by priority (lower number = higher preference):

1. **Redis (priority 50)** — preferred when configured
2. **Memory (priority 100)** — fallback when Redis is not available

If only one provider pack is deployed, that provider is used regardless of priority.

## Deployment Patterns

### Development (default)

```
demo-bundle/
└── providers/
    └── state-memory.gtpack    # auto-selected, no setup
```

### Production

```
deploy-bundle/
└── providers/
    ├── state-redis.gtpack     # preferred (priority 50)
    └── state-memory.gtpack    # fallback if Redis setup fails
```

### Redis-only

```
deploy-bundle/
└── providers/
    └── state-redis.gtpack     # only option, must be configured
```
