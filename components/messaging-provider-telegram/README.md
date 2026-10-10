# Messaging Provider Telegram Component

Provider-core Telegram messaging provider.

## Component ID
- `messaging-provider-telegram`

## Provider types
- `messaging.telegram.bot`

## Secrets
- `TELEGRAM_BOT_TOKEN` (tenant): Telegram bot token used for sendMessage requests.

## Verified caller (per-end-user ledger)

`ingest_http` stamps `extensions.caller = {user_verified: true, sub: "<telegram user id>", iss: "telegram"}`
on the envelope only when ALL of these hold:

- greentic-start marked the request as authenticated: the reserved header
  `x-greentic-auth-verified: telegram` is on the `ingest_http` input. The host
  adds it only after the `x-telegram-bot-api-secret-token` header matched the
  endpoint's `webhook_secret_ref`, and strips any client-supplied copy first.
  An endpoint with no `webhook_secret_ref` (legacy) is still served, but gets no
  caller. Run `greentic-deployer op messaging endpoint rotate-webhook-secret`
  on it to provision one.
- the update is a `message` or `callback_query` with a `from` that is not a bot;
- the chat is private (`chat.type == "private"`; for a callback, the chat of the
  message the button sits on) and `from.id` equals the chat id;
- `from.id` is a positive JSON integer (anything else is omitted).

Groups, supergroups and channels never get a caller: a person's private history
must not be injected into a reply a group can read. Never trust the marker from
anywhere but the host; the provider cannot verify the secret itself.
