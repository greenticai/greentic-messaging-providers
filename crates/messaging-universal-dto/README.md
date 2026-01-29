# messaging-universal-dto

Shared DTOs for the universal operator-provider protocol v1.

This crate defines the JSON shapes that every messaging provider implements:
- `HttpInV1` / `HttpOutV1` for normalized webhook ingress.
- `RenderPlanInV1` / `EncodeInV1` / `ProviderPayloadV1` for planning and encoding send payloads.
- `SendPayloadInV1` plus `SendPayloadResultV1` for delivering the encoded bytes.

The `HttpOutV1.events` vector uses `greentic_types::ChannelMessageEnvelope` so webhook normalization can describe inbound messaging while staying deterministic.
