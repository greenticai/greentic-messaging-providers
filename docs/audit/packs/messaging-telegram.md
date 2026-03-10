# Pack Audit: messaging-telegram (v0.4.34)

## Overview

| Field | Value |
|-------|-------|
| Pack ID | messaging-telegram |
| Version | 0.4.34 |
| Provider Type | messaging.telegram.bot |
| Components | 2 |
| Flows | 2 (setup_default, requirements) |
| Ingress | Yes (separate WASM) |
| Secrets | TELEGRAM_BOT_TOKEN |

## Extensions

- `greentic.ext.capabilities.v1` — capability offer `messaging-telegram-v1`
- `greentic.provider-extension.v1` — provider type, ops, runtime binding

## Status

Migrated to capability-driven pattern. Legacy flows and component stubs removed.
