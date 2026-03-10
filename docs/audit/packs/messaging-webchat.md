# Pack Audit: messaging-webchat (v0.4.34)

## Overview

| Field | Value |
|-------|-------|
| Pack ID | messaging-webchat |
| Version | 0.4.34 |
| Provider Type | messaging.webchat |
| Components | 1 |
| Flows | 2 (setup_default, requirements) |
| Ingress | Inline (same component) |
| Secrets | jwt_signing_key |

## Extensions

- `greentic.ext.capabilities.v1` — capability offer `messaging-webchat-v1`
- `greentic.provider-extension.v1` — provider type, ops, runtime binding

## Status

Migrated to capability-driven pattern. Legacy flows and component stubs removed.
