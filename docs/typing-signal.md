# Typing signal (`send_typing`)

An OPTIONAL provider op that shows the channel's native "the bot is typing"
indicator while a turn runs. The cross-repo design lives in greentic-designer
(`docs/superpowers/specs/2026-09-29-thinking-and-typing-indicators-design.md`,
§5); this doc records what this repo owns.

A provider opts in by listing `send_typing` in its pack's
`greentic.provider-extension.v1` ops (both `pack.yaml` and
`pack.manifest.json`). The host never calls an op that is not listed.
`crates/provider-tests/tests/typing_op_allowlist.rs` pins exactly which
providers may list it.

## Contract as implemented

Shared types: `provider_common::typing`.

Input (JSON, unknown keys ignored — the host's `tenant` hint is accepted and
not read):

```json
{
  "v": 1,
  "provider_type": "messaging.webchat",
  "tenant_id": "acme",
  "message": { "...": "the INBOUND ChannelMessageEnvelope the turn answers" },
  "config": { "...": "the provider config the host hands ingest_http" }
}
```

Output:

```json
{ "v": 1, "ok": true, "refresh_after_ms": 4000 }
```

- Every failure is `{ "v": 1, "ok": false, "error": "…" }` — never a trap,
  never a retry, never a state change. The host treats it as "no typing this
  time"; a turn is never affected.
- `refresh_after_ms` is the platform's own indicator lifetime; the host
  re-sends every `clamp(refresh_after_ms − 500 ms, 1 s, 30 s)`.
- No implementation posts a visible message or writes history.
- Webchat adds `_greentic { tenant, conversation_id, watermark_bumped }`, the
  same block `send_payload` emits, so the host can fire the WebSocket notify.

## Providers

| Provider type | Does | `refresh_after_ms` | Target read from the inbound envelope |
|---|---|---|---|
| `messaging.webchat`, `messaging.webchat-gui`, `messaging.3aigent-gui` | ephemeral typing activity (below) | 4000 | `session_id`; env/tenant/team from `metadata`, else `tenant` |
| `messaging.teams` (Bot Framework pack, `messaging-teams/build_pack.sh`) | `{"type":"typing"}` on the connector path | 3000 | `metadata.service_url`/`serviceUrl` (else config `default_service_url`), `metadata.conversation_id`/`conversationId`, else `session_id` |
| `messaging.telegram.bot` | `sendChatAction` `action=typing` | 4500 | `to[0].id`, else `metadata.chat_id`, else `session_id`; optional numeric `metadata.message_thread_id` |
| `messaging.whatsapp.cloud` | Cloud API `typing_indicator` | 20000 | `metadata.wa_message_id` |

Not advertised: Slack, Webex, email, dummy, and `messaging.teams.graph` (the
Graph pack shares the Teams component but Graph has no typing API; the op
refuses any setup mode other than `bot_framework`).

Each provider's "no conversation" fallback session (`webchat`, `teams`,
`telegram`) is refused rather than typed into.

Teams: Bot Framework shows typing in 1:1 and group chats; in channels it is
accepted and not displayed.

Telegram: the bot token is part of every API URL, so it is redacted from any
error string.

## Webchat: typing is never history

`send_typing` writes a separate state key, `<conv_key>:typing`, holding
`{ since_watermark, until_ms }` (`until_ms = now + 5 s`). It never rewrites the
conversation document: the state store has no compare-and-swap, and
`send_payload` appending the reply concurrently would otherwise risk losing the
reply.

`GET …/activities` appends one synthesized `{ "type": "typing", "from":
{ "role": "bot" } }` activity (fresh id per poll) while the slot is live and no
bot activity (`message` or the `event` error card) has been stored at or after
`since_watermark`. A user message after the raise does not hide it. No
watermark is consumed: the activity carries the current `next_watermark`, so the
WebSocket pump still delivers it and the reply later takes that watermark. A
reconnect after the slot lapsed, or after the reply, replays no typing. An
unreadable slot is "no typing", never a failed poll.

## WhatsApp

Meta's typing indicator must name the inbound message, so `ingest_http` now
stamps `metadata.wa_message_id` (the Cloud API `wamid.…`). An inbound envelope
from an older provider build has none, and `send_typing` answers `ok: false`.
The call also marks that message read, and uses the provider's configured
`api_version` (default `v19.0`); Meta's docs name no minimum version.

## What the host must do

- Send `config` (Teams and WhatsApp need it; the others tolerate its absence).
- Call the op only when the pack lists it.
- Forward `_greentic` from a webchat `send_typing` result to the WebSocket
  notifier, as it does for `send_payload`.
- Stop refreshing before the first reply's `send_payload`.
