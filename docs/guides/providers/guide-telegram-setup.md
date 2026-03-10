# Telegram Provider Setup Guide

Set up the Telegram messaging provider with `greentic-operator` demo mode to send and receive messages through Telegram Bot API.

## Architecture

```
                    EGRESS (send message)
                    ====================
gtc op demo send
    |
    v
[messaging-telegram.gtpack]  -- WASM provider component
    |  render_plan -> encode -> send_payload
    v
Telegram Bot API  (POST /bot{token}/sendMessage)
    |
    v
Message appears in Telegram chat


                    INGRESS (receive message)
                    =========================
User sends message to bot in Telegram
    |
    v
Telegram Cloud  -- POST webhook -->  Operator HTTP gateway (:8080)
                                         |
                                         v
                                    [messaging-ingress-telegram.wasm]
                                         |  parse update, extract chat/text
                                         v
                                    ChannelMessageEnvelope (normalized)
```

Key details:

- Egress uses the Telegram Bot API (`sendMessage` endpoint) for outbound messages.
- Ingress uses a separate WASM component (`messaging-ingress-telegram.wasm`) to parse Telegram webhook updates.
- Supports `reply_to_message_id` threading for reply context.
- Adaptive Cards are downsampled to plain text (Telegram has no native AC support).
- The bot token is read from the secrets store at key `telegram_bot_token`.

---

## Prerequisites

| Requirement | Notes |
|-------------|-------|
| `greentic-operator` binary | v0.4.26+ installed at `/root/.cargo/bin/greentic-operator` |
| `messaging-telegram.gtpack` | In `demo-bundle/providers/messaging/` |
| Telegram bot | Created via @BotFather (bot token required) |
| `seed_all` tool | For writing encrypted secrets to dev store |
| Public URL for webhooks | Cloudflared tunnel auto-starts with operator |

---

## Step 1: Create a Telegram Bot

1. Open Telegram and message [@BotFather](https://t.me/BotFather).
2. Send `/newbot` and follow the prompts (choose a name and username).
3. Copy the **bot token** from BotFather's response. Format: `123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11`.

### Get your chat ID

To test egress, you need the chat ID of the conversation where the bot will send messages.

1. Open a chat with your new bot in Telegram and send any message.
2. Call the `getUpdates` endpoint:

```bash
BOT_TOKEN="<your-bot-token>"

curl -s "https://api.telegram.org/bot${BOT_TOKEN}/getUpdates" | python3 -m json.tool
```

3. Find the `chat.id` value in the response:

```json
{
  "ok": true,
  "result": [{
    "update_id": 201762148,
    "message": {
      "from": { "id": 7951102355, "first_name": "YourName" },
      "chat": { "id": 7951102355, "type": "private" },
      "text": "hello"
    }
  }]
}
```

Note the `chat.id` value -- you will use it as the `--to` target when sending messages.

---

## Step 2: Seed Secrets

The provider reads the bot token from the secrets store. Two keys are needed for compatibility:

| Secret Key | Description |
|------------|-------------|
| `telegram_bot_token` | Primary key used by `send_payload` |
| `bot_token` | Compatibility alias used by setup flows |

### Batch seeding (required)

Due to a DEK cache bug in `greentic-secrets-core` v0.4.22, all secrets for a given category must be written in a single session. Writing secrets in separate sessions causes each to get a different data encryption key, making previously written secrets unreadable.

```bash
GREENTIC_ENV=dev ./tools/seed_all/target/release/seed_all \
  demo-bundle/.greentic/dev/.dev.secrets.env \
  "secrets://dev/default/_/messaging-telegram/telegram_bot_token" "<your-bot-token>" \
  "secrets://dev/default/_/messaging-telegram/bot_token" "<your-bot-token>"
```

Secret URI pattern: `secrets://dev/default/_/messaging-telegram/<key_name>`

---

## Step 3: Verify the gtpack

Confirm the pack file is in the correct location:

```bash
ls -la demo-bundle/providers/messaging/messaging-telegram.gtpack
```

Expected bundle layout:

```
demo-bundle/
  greentic.demo.yaml
  providers/
    messaging/
      messaging-telegram.gtpack
  .greentic/
    dev/
      .dev.secrets.env
```

---

## Step 4: Start the Operator

```bash
GREENTIC_ENV=dev gtc op demo start \
  --bundle demo-bundle \
  --tenant default \
  --env dev
```

The operator starts:
- HTTP ingress gateway on `http://127.0.0.1:8080`
- Cloudflared tunnel (auto-started) for external webhook callbacks

Get the tunnel URL from the cloudflared log:

```bash
TUNNEL=$(grep -oP 'https://[a-z0-9-]+\.trycloudflare\.com' demo-bundle/logs/cloudflared.log | tail -1)
echo "$TUNNEL"
```

---

## Step 5: Register the Telegram Webhook

Telegram needs a public URL to deliver incoming message updates to your operator.

### Set the webhook

```bash
BOT_TOKEN="<your-bot-token>"

curl -s "https://api.telegram.org/bot${BOT_TOKEN}/setWebhook?url=${TUNNEL}/v1/messaging/ingress/messaging-telegram/default/default"
```

### Verify the webhook

```bash
curl -s "https://api.telegram.org/bot${BOT_TOKEN}/getWebhookInfo" | python3 -m json.tool
```

Expected output:

```json
{
  "ok": true,
  "result": {
    "url": "https://abc-xyz.trycloudflare.com/v1/messaging/ingress/messaging-telegram/default/default",
    "has_custom_certificate": false,
    "pending_update_count": 0
  }
}
```

Check that `url` matches your tunnel and `pending_update_count` is 0 (or decreasing). If `last_error_message` is present, see the Troubleshooting section.

### Remove webhook (cleanup)

```bash
curl -s "https://api.telegram.org/bot${BOT_TOKEN}/deleteWebhook"
```

---

## Step 6: Test Egress (Send a Message)

```bash
GREENTIC_ENV=dev gtc op demo send \
  --bundle demo-bundle \
  --provider messaging-telegram \
  --to "7951102355" \
  --text "Hello from Greentic" \
  --tenant default
```

Replace `7951102355` with your chat ID from Step 1.

Expected output on success:

```json
{
  "ok": true,
  "message": null,
  "retryable": false
}
```

The message should appear in the Telegram chat with the bot.

### Send pipeline breakdown

The operator invokes three WASM operations in sequence:

1. **render_plan** -- Determines rendering tier. Telegram does not support Adaptive Cards, so AC content is downsampled to plain text with a summary.
2. **encode** -- Serializes the message into a provider payload. For non-AC content, encodes the text directly. For AC content, `extract_ac_summary()` replaces the text with the downsampled AC summary at encode time.
3. **send_payload** -- Decodes the payload, resolves the `telegram_bot_token` from secrets, and POSTs to `https://api.telegram.org/bot{token}/sendMessage`.

---

## Step 7: Test Ingress (Simulated Webhook)

Post a synthetic Telegram update to the operator gateway to test the ingress pipeline without a live webhook:

```bash
curl -X POST http://localhost:8080/v1/messaging/ingress/messaging-telegram/default/default \
  -H "Content-Type: application/json" \
  -d '{
    "update_id": 1,
    "message": {
      "message_id": 1,
      "from": {"id": 123, "is_bot": false, "first_name": "TestUser"},
      "chat": {"id": 123, "type": "private"},
      "date": 1234567890,
      "text": "hello from simulated webhook"
    }
  }'
```

If the echo bot pipeline is configured, this triggers the full loop: ingress parses the update, the flow runs, and `send_payload` sends a reply back to chat ID 123.

---

## Step 8: Test Ingress (Live Webhook)

With the webhook registered (Step 5) and the operator running (Step 4):

1. Open Telegram and send a message to your bot.
2. Telegram delivers the update to the cloudflared tunnel URL.
3. The operator routes the request to `messaging-ingress-telegram.wasm`.
4. The ingress component parses the update into a `ChannelMessageEnvelope`.
5. If an echo bot flow is configured, the operator runs the pipeline and sends a reply back through the Telegram Bot API.

Check operator logs for request processing output.

---

## Telegram Bot API Quick Reference

### Get bot info

```bash
curl -s "https://api.telegram.org/bot${BOT_TOKEN}/getMe" | python3 -m json.tool
```

### Get pending updates (polling mode, useful for debugging)

```bash
curl -s "https://api.telegram.org/bot${BOT_TOKEN}/getUpdates" | python3 -m json.tool
```

Note: `getUpdates` does not work when a webhook is set. Remove the webhook first with `deleteWebhook` if you need to poll manually.

### Get webhook status

```bash
curl -s "https://api.telegram.org/bot${BOT_TOKEN}/getWebhookInfo" | python3 -m json.tool
```

---

## Troubleshooting

### Webhook not receiving messages

| Symptom | Likely Cause | Fix |
|---------|-------------|-----|
| `pending_update_count` keeps growing | Webhook URL unreachable | Verify tunnel is running; re-register webhook with new URL |
| `last_error_message` shows DNS error | Cloudflared tunnel URL changed | Tunnel gets a new random URL on every restart; re-register webhook |
| `last_error_message` shows 404 | Transient DNS propagation delay | Wait 1-2 minutes and retry `setWebhook` |
| No `last_error_message` but no updates | Bot has not received any messages | Send a message to the bot first |

### "missing secret: telegram_bot_token"

The bot token is not in the secrets store. Re-run the seed step (Step 2). Ensure `GREENTIC_ENV=dev` is set when running the operator.

### Egress returns error but no message sent

Check that:
- The bot token is valid (test with `getMe`)
- The chat ID is correct (the user must have messaged the bot at least once to open a private chat)
- The bot has not been blocked by the user

### Ingress uses a separate WASM component

The ingress handler runs in `messaging-ingress-telegram.wasm`, which is separate from the main `messaging-provider-telegram.wasm` used for egress. Both are bundled inside `messaging-telegram.gtpack`.

### DEK cache bug

`greentic-secrets-core` v0.4.22 shares a single DEK cache slot per `(env, tenant, team, category)`. Writing secrets in separate sessions causes each to get a different data encryption key. Always batch-seed all secrets for the `messaging-telegram` category in a single session.

### getUpdates returns empty after setting webhook

This is expected. Telegram disables `getUpdates` when a webhook is active. Use `deleteWebhook` to switch back to polling mode for debugging.

---

## Supported Operations

The `messaging-provider-telegram` WASM component exposes these operations:

| Operation | Description |
|-----------|-------------|
| `send` | Send a message to a Telegram chat |
| `reply` | Reply in a thread (uses `reply_to_message_id`) |
| `render_plan` | Determine rendering tier (plain text; AC is downsampled) |
| `encode` | Encode a universal message into a Telegram-specific payload |
| `send_payload` | Execute the encoded payload against Telegram Bot API |
| `ingest_http` | Parse incoming Telegram webhook update into `ChannelMessageEnvelope` |
| `qa-spec` | Return setup questions for the provider |
| `apply-answers` | Apply QA answers to produce provider config |
| `i18n-keys` | Return i18n key catalog |
