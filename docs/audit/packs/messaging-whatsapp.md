# Pack Audit: messaging-whatsapp (v0.4.34)

## Overview

| Field | Value |
|-------|-------|
| Pack ID | messaging-whatsapp |
| Version | 0.4.34 |
| Provider Type | messaging.whatsapp.cloud |
| Components | 2 |
| Flows | 2 (setup_default, requirements) |
| Ingress | Yes (separate WASM) |
| Secrets | WHATSAPP_TOKEN, WHATSAPP_VERIFY_TOKEN (optional), WHATSAPP_PHONE_NUMBER_ID |

## Extensions

- `greentic.ext.capabilities.v1` — capability offer `messaging-whatsapp-v1`
- `greentic.provider-extension.v1` — provider type, ops, runtime binding

## Status

Migrated to capability-driven pattern. Legacy flows and component stubs removed.
