# Pack Audit: messaging-teams (v0.4.34)

## Overview

| Field | Value |
|-------|-------|
| Pack ID | messaging-teams |
| Version | 0.4.34 |
| Provider Type | messaging.teams.bot |
| Components | 2 |
| Flows | 2 (setup_default, requirements) |
| Ingress | Yes (separate WASM) |
| Secrets | MS_GRAPH_TENANT_ID, MS_GRAPH_CLIENT_ID, MS_GRAPH_REFRESH_TOKEN |

## Extensions

- `greentic.ext.capabilities.v1` — capability offer `messaging-teams-v1`
- `greentic.provider-extension.v1` — provider type, ops, runtime binding

## Status

Migrated to capability-driven pattern. Legacy flows and component stubs removed.
