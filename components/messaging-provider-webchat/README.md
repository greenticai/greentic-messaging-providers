# Messaging Provider Webchat Component

Provider-core WebChat messaging provider (send + ingest).

## Component ID
- `messaging-provider-webchat`

## Provider types
- `messaging.webchat`

## Secrets
- None.

## Host requirements
- Imports `greentic:state/state-store@1.1.0` (`write-if-absent`), used to claim a Direct Line activity slot atomically. A host that serves only `@1.0.0` cannot instantiate this component; publish the pack only after every host serves `@1.1.0`.
- `messaging-provider-webchat-gui` and `messaging-provider-3aigent-gui` share the same Direct Line code and carry the same requirement.
