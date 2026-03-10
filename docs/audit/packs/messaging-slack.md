# Pack Audit: messaging-slack (v0.4.34)

## Overview

| Field | Value |
|-------|-------|
| Pack ID | messaging-slack |
| Version | 0.4.34 |
| Provider Type | messaging.slack.api |
| Components | 2 |
| Flows | 2 (setup_default, requirements) |
| Ingress | Yes (separate WASM) |
| Secrets | SLACK_BOT_TOKEN, SLACK_SIGNING_SECRET (optional) |

## Extensions

- `greentic.ext.capabilities.v1` — capability offer `messaging-slack-v1`
- `greentic.provider-extension.v1` — provider type, ops, runtime binding

## Status

Migrated to capability-driven pattern. Legacy flows and component stubs removed.
