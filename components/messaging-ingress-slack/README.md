# Messaging Ingress Slack Component

Ingress-only Slack component for webhook validation and normalization.

## Component ID
- `messaging-ingress-slack`

## Provider types
- `messaging.slack.api`

## Secrets
- `SLACK_SIGNING_SECRET` (tenant): Slack signing secret used to verify webhook signatures (optional). When set, a request whose `X-Slack-Request-Timestamp` is more than 300 s from now (past or future) is refused, and signatures are compared in constant time (`slack-auth-core`).
