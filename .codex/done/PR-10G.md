# PR-10G.md (greentic-messaging-providers)
# Title: Slack provider-core pack (send + reply) + schema + fixtures

## Goal
Add Slack as provider-core messaging provider.

## provider_type
- `messaging.slack.api`

## Ops
- `send`
- `reply` (optional but recommended)

## Schema
- `schemas/messaging/slack/config.schema.json`:
  - `bot_token` (x-secret)
  - `signing_secret` (x-secret, optional for ingest later)
  - `default_channel` (optional)
  - `api_base_url` (default https://slack.com/api)

## Runtime behavior
- invoke("send"):
  - maps SendInput to Slack chat.postMessage
  - supports:
    - `to.kind=channel|user` with `to.id`
    - `text`
    - `rich.format=slack_blocks` → blocks payload
- invoke("reply"):
  - uses `thread_id` / `reply_to_id` mapped to Slack thread_ts

## Tests
- mocked HTTP (no live Slack)
- contract tests: request JSON matches Slack expected shapes
- pack validation tests

## Acceptance
- Pack self-describing; send works with HTTP mock.
