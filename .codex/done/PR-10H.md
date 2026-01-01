# PR-10H.md (greentic-messaging-providers)
# Title: Webex provider-core pack (send) + schema + fixtures

## provider_type
- `messaging.webex.bot`

## Schema
- `schemas/messaging/webex/config.schema.json`:
  - `access_token` (x-secret)
  - `default_room_id` (optional)
  - `api_base_url` (default https://webexapis.com/v1)

## Ops
- `send`

## Runtime behavior
- invoke("send"):
  - posts to Webex Messages API
  - supports:
    - roomId/personId via `to.kind=room|user`
    - markdown/text
    - attachments (if supported; else return clear error)

## Tests
- mocked HTTP, pack validation

## Acceptance
- Works with provider-core send contract.
