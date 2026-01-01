# PR-10J.md (greentic-messaging-providers)
# Title: WebChat provider-core pack (send + ingest) + schema + fixtures

## Goal
Support Greentic’s own webchat / channel_webhook / UI chat surfaces.

## provider_type
- `messaging.webchat`

## Schema
- `schemas/messaging/webchat/config.schema.json`:
  - `route` or `tenant_channel_id`
  - `mode`: `local_queue|websocket|pubsub` (depending on your channel architecture)
  - `base_url` (optional) for hosted push

## Ops
- `send`
- `ingest` (optional; normalize inbound webchat events to MessageEnvelope)

## Runtime behavior
- send:
  - writes to state-store queue OR emits event (depending on runner infra)
- ingest:
  - takes raw webchat message payload and returns normalized envelope JSON

## Tests
- deterministic: send writes to state-store; ingest normalizes fixture payload

## Acceptance
- Enables end-to-end web UI chat flows via provider-core without external services.
