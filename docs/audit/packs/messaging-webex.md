# Pack Audit: messaging-webex (v0.4.34)

## Overview

| Field | Value |
|-------|-------|
| Pack ID | messaging-webex |
| Version | 0.4.34 |
| Provider Type | messaging.webex.bot |
| Components | 1 |
| Flows | 2 (setup_default, requirements) |
| Ingress | No |
| Secrets | WEBEX_BOT_TOKEN |

## Extensions

- `greentic.ext.capabilities.v1` — capability offer `messaging-webex-v1`
- `greentic.provider-extension.v1` — provider type, ops, runtime binding

## Status

Migrated to capability-driven pattern. Legacy flows and component stubs removed.
