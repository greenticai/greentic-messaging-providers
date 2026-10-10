# Messaging Provider Webex Component

Provider-core Webex messaging provider (messages API).

## Component ID
- `messaging-provider-webex`

## Provider types
- `messaging.webex.bot`

## Secrets
- `WEBEX_BOT_TOKEN` (tenant): Webex bot access token used for Messages API calls.

## Webhook verification and the verified caller

Inbound webhooks are verified with HMAC-SHA1 over the raw body
(`x-spark-signature` / `x-webex-signature`) using `WEBEX_WEBHOOK_SECRET`.
Admission is unchanged: with no secret configured the webhook is admitted
unverified (a warning is logged and `setup_webhook` reports
`signing_secret_missing`); with a secret, a missing or wrong signature is
rejected with 401.

Only a request whose signature was actually verified can carry a caller. The
provider then stamps `extensions.caller = {user_verified: true, sub: <personId>,
iss: "webex"}` when, additionally, the event is `messages.created`, the sender
comes from the message fetched from the Webex API (not the webhook body), is not
a `@webex.bot` account, and the space is 1:1 (`roomType == "direct"`). `sub` is
the opaque Webex `personId`, never the email. Group spaces, memberships,
attachment actions, fetch failures and unsigned deployments carry no caller.
