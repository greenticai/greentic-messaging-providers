# Messaging Provider Slack Component

Provider-core Slack messaging provider (chat.postMessage).

## Component ID
- `messaging-provider-slack`

## Provider types
- `messaging.slack.api`

## Secrets
- `SLACK_BOT_TOKEN` (tenant): Slack bot token used for chat.postMessage calls.
- `SLACK_SIGNING_SECRET` (tenant): Slack signing secret (optional for future webhook validation).

## Inbound files
A `file_share` message carries `event.files[]`. The provider never downloads them: for each file on a `*.slack.com` https host it emits `attachments[i]` (url null) plus `extensions["attachment_fetch"][i] = {kind: "bearer", url, secret_key: "SLACK_BOT_TOKEN"}`, and the host fetches with the bot token. The bot token needs the `files:read` scope; an app installed without it gets `403`/HTML from Slack on download and the host drops the attachment. The legacy `messaging-ingress-slack` component emits the same shape.
