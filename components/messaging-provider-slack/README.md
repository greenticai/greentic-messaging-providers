# Messaging Provider Slack Component

Provider-core Slack messaging provider (chat.postMessage).

## Component ID
- `messaging-provider-slack`

## Provider types
- `messaging.slack.api`

## Secrets
- `SLACK_BOT_TOKEN` (tenant): Slack bot token used for chat.postMessage calls.
- `SLACK_SIGNING_SECRET` (tenant): Slack signing secret. `ingest_http` verifies `X-Slack-Signature` with it (shared `slack-auth-core`, 300 s replay window); a request that does not verify is still ingested but carries no caller identity.

## Verified caller

For a request the provider itself authenticated, a 1:1 Slack conversation (`channel_type == "im"`, or a `D…` channel when no type is sent) gets `extensions.caller = {"user_verified": true, "sub": "<slack user id>", "iss": "slack:<team>"}` on the envelope: plain messages, `block_actions` and `view_submission`. `<team>` is the Enterprise Grid `enterprise_id`, else the workspace `team_id`; a user whose own team (`event.user_team` / `user.team_id`) differs from the workspace (Slack Connect) uses that team. Bot events, events without a user, edits/deletions, and everything in channels, private groups and multi-party DMs get no caller, so a person's private history is never injected into a reply others can read. Malformed ids omit the block.
