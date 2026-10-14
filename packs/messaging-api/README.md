# Messaging API Pack

Programmatic channel: send by REST, receive by SSE stream, polling or webhook.

## Pack ID
- `messaging-api`

## Providers
- `messaging.api` (ops: send, ingest_http, poll_events, render_plan, encode, send_payload, qa-spec, apply-answers, i18n-keys)

## Routes (`ingest_http`)
- `POST /v1/messaging/api/{tenant}/messages` - body `{conversation_id?, client_message_id, text | card}`, answers `202 {conversation_id, message_id}`
- `GET  /v1/messaging/api/{tenant}/messages?conversation_id=&after=` - polling, answers `{events, next_cursor}`
- `GET  /v1/messaging/api/{tenant}/conversations/{id}/stream` (declared as `conversations/{path*}`) - auth-only probe used by greentic-start, which serves the SSE stream itself; answers `200` for a good key and `401` otherwise, touches no state
- Both require `Authorization: Bearer <api key>`; failures are a bare `{"error":{"code":..}}`.

## Event log
One log per conversation with a monotonic `u64` cursor, built on the state-store
`write-if-absent` (no compare-and-swap is needed). The provider op `poll_events`
(`{"conversation_id","since"}` -> `{"events","next_cursor"}`) is what greentic-start
reads to serve SSE. Fixtures: `docs/fixtures/api-channel/`.

## Secrets
- `API_CHANNEL_API_KEY`, `API_CHANNEL_WEBHOOK_SECRET`. Never logged, never in an event or a pack file.
