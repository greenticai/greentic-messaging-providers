# Webex Provider Setup Guide

Set up the Webex messaging provider with `greentic-operator` demo mode to send and receive messages through Webex Spaces (rooms).

## Architecture

```
EGRESS (outbound)
gtc op demo send
  -> messaging-provider-webex.wasm (render_plan -> encode -> send_payload)
  -> POST https://webexapis.com/v1/messages
  -> Message appears in Webex room/DM

INGRESS (inbound)
User sends message in Webex
  -> Webex webhook POST to your tunnel URL
  -> Operator HTTP gateway (port 8080)
  -> ingest_http (parses webhook, fetches message via GET /messages/{id})
  -> ChannelMessageEnvelope
```

Key details:

- Webex webhooks only deliver the message ID, not the content. The ingress handler makes a secondary `GET /messages/{id}` call to retrieve the actual text.
- Adaptive Card v1.3 is supported natively via Webex attachments (no downsampling needed).
- Destination auto-detection: `Y2lz*` prefix = roomId, `@` = email address.
- Bot token is read from the secrets store at key `WEBEX_BOT_TOKEN`.

## Prerequisites

- `greentic-operator` binary installed (`/root/.cargo/bin/greentic-operator`)
- `messaging-webex.gtpack` in `demo-bundle/providers/messaging/`
- Demo bundle directory with secrets store initialized
- `jq` and `curl` for webhook management commands

## Step 1: Create a Webex Bot

1. Go to https://developer.webex.com/my-apps/new/bot
2. Fill in bot name, username, icon, and description
3. After creation, copy the **bot access token** -- this is shown only once
4. Add the bot to a Webex Space (room) where you want to send/receive messages

To find the room ID after adding the bot:

```bash
WEBEX_TOKEN="<your-bot-token>"

curl -s "https://webexapis.com/v1/rooms" \
  -H "Authorization: Bearer $WEBEX_TOKEN" | jq '.items[] | {id, title, type}'
```

Note the `id` value for the room you want to use.

## Step 2: Seed Secrets

The provider reads the bot token from the secrets store at key `WEBEX_BOT_TOKEN`. The setup flow also expects `bot_token` for compatibility.

Use the `seed_all` binary to batch-seed secrets (required due to the DEK cache bug in greentic-secrets-core -- all secrets in the same category must be written in a single session):

```bash
# Seed both keys in the messaging-webex category
GREENTIC_ENV=dev ./tools/seed_all \
  demo-bundle/.greentic/dev/.dev.secrets.env \
  "secrets://dev/default/_/messaging-webex/webex_bot_token" "<your-bot-token>" \
  "secrets://dev/default/_/messaging-webex/bot_token" "<your-bot-token>"
```

Secret URI pattern: `secrets://dev/default/_/messaging-webex/<key_name>`

## Step 3: Verify the gtpack

Confirm the pack file exists:

```bash
ls -la demo-bundle/providers/messaging/messaging-webex.gtpack
```

Expected bundle layout:

```
demo-bundle/
  greentic.demo.yaml
  providers/
    messaging/
      messaging-webex.gtpack
  .greentic/
    dev/
      .dev.secrets.env
```

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

Get the tunnel URL:

```bash
TUNNEL=$(grep -oP 'https://[a-z0-9-]+\.trycloudflare\.com' demo-bundle/logs/cloudflared.log | tail -1)
echo "$TUNNEL"
```

## Step 5: Register the Webex Webhook

Webex must know where to send incoming message events. The target URL follows this pattern:

```
{TUNNEL_URL}/v1/messaging/ingress/messaging-webex/default/default
```

### List existing webhooks

```bash
curl -s "https://webexapis.com/v1/webhooks" \
  -H "Authorization: Bearer $WEBEX_TOKEN" | jq '.items[] | {id, name, targetUrl, status}'
```

### Create a new webhook

```bash
curl -X POST "https://webexapis.com/v1/webhooks" \
  -H "Authorization: Bearer $WEBEX_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"name\": \"greentic\",
    \"targetUrl\": \"${TUNNEL}/v1/messaging/ingress/messaging-webex/default/default\",
    \"resource\": \"messages\",
    \"event\": \"created\"
  }"
```

### Update an existing webhook

When the tunnel URL changes (it changes on every operator restart), update the existing webhook instead of creating a new one:

```bash
WEBHOOK_ID="<id-from-list-above>"

curl -X PUT "https://webexapis.com/v1/webhooks/${WEBHOOK_ID}" \
  -H "Authorization: Bearer $WEBEX_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"name\": \"greentic\",
    \"targetUrl\": \"${TUNNEL}/v1/messaging/ingress/messaging-webex/default/default\"
  }"
```

### Verify webhook status

```bash
curl -s "https://webexapis.com/v1/webhooks" \
  -H "Authorization: Bearer $WEBEX_TOKEN" | jq '.items[] | select(.name=="greentic") | {id, status, targetUrl}'
```

The `status` field should be `active`. If it shows `disabled`, the targetUrl was unreachable when Webex last tried to deliver an event.

## Step 6: Test Egress (Send a Message)

### Send to a room by ID

```bash
GREENTIC_ENV=dev gtc op demo send \
  --bundle demo-bundle \
  --provider messaging-webex \
  --to "Y2lzY29zcGFyazovL3...roomId..." \
  --text "Hello from Greentic" \
  --tenant default
```

### Send to a person by email

```bash
GREENTIC_ENV=dev gtc op demo send \
  --bundle demo-bundle \
  --provider messaging-webex \
  --to "user@example.com" \
  --text "Hello from Greentic" \
  --tenant default
```

Destination auto-detection rules:
- Starts with `Y2lz` -> treated as a Webex roomId
- Contains `@` -> treated as an email address
- Otherwise -> treated as an email address

A successful response includes `"ok": true` and a `message_id`.

## Step 7: Test Ingress (Receive a Message)

### Live test

Send a message to the bot in the Webex Space. The webhook fires, and the operator logs should show the ingest pipeline processing the event.

### Simulated test

```bash
curl -X POST http://localhost:8080/v1/messaging/ingress/messaging-webex/default/default \
  -H "Content-Type: application/json" \
  -d '{
    "resource": "messages",
    "event": "created",
    "data": {
      "id": "Y2lzY29zcGFyazovL3VzL01FU1NBR0UvMTIzNDU2",
      "roomId": "Y2lzY29zcGFyazovL3VzL1JPT00vYWJjZGVm",
      "personId": "Y2lzY29zcGFyazovL3VzL1BFT1BMRS91c2VyMTIz",
      "personEmail": "user@example.com",
      "created": "2026-01-01T00:00:00.000Z"
    }
  }'
```

Note: Simulated ingress will return a 502 for the secondary GET call since the message ID is fake. For a full E2E test, use real messages sent to the bot.

## Webex API Quick Reference

### List rooms the bot belongs to

```bash
curl -s "https://webexapis.com/v1/rooms" \
  -H "Authorization: Bearer $WEBEX_TOKEN" | jq '.items[] | {id, title, type}'
```

### Get a specific message

```bash
curl -s "https://webexapis.com/v1/messages/{messageId}" \
  -H "Authorization: Bearer $WEBEX_TOKEN" | jq .
```

### List recent messages in a room

```bash
curl -s "https://webexapis.com/v1/messages?roomId={roomId}&max=5" \
  -H "Authorization: Bearer $WEBEX_TOKEN" | jq '.items[] | {id, text, personEmail, created}'
```

## Troubleshooting

### "missing secret: WEBEX_BOT_TOKEN"

The bot token is not in the secrets store. Re-run the seed step (Step 2). Ensure `GREENTIC_ENV=dev` is set.

### Ingress returns 502

The secondary `GET /messages/{id}` call failed. This happens when:
- The bot token is missing or expired
- The message ID in the webhook payload is invalid (simulated test)
- Network connectivity issue to `webexapis.com`

The webhook envelope is still created with empty text and a `webex.ingestError` metadata field.

### "invalid config: unknown field" / deny_unknown_fields

The `ProviderConfig` struct uses `#[serde(deny_unknown_fields)]`. Only these fields are accepted in the config object:
- `enabled` (bool, default: true)
- `public_base_url` (string, required)
- `default_room_id` (string, optional)
- `default_to_person_email` (string, optional)
- `api_base_url` (string, optional, default: `https://webexapis.com/v1`)
- `bot_token` (string, optional -- overrides secrets store)

Do not pass extra fields like `provider_id` or `webhook_url` in the config.

### Webhook shows "disabled" status

Webex disables webhooks when the target URL is unreachable. This happens every time the cloudflared tunnel URL changes. Update the webhook (Step 5) with the new tunnel URL.

### Adaptive Card not rendering

Webex supports AC up to v1.3. The provider automatically caps the version field. Ensure the card JSON has a valid `body` array. The `markdown` field is sent alongside the attachment as a fallback for clients that cannot render cards.

### Ingress hardcodes env=default, tenant=default

The ingress handler creates `ChannelMessageEnvelope` with hardcoded `env=default` and `tenant=default`. This matches the default demo setup. The URL path segments (`/default/default`) control routing at the operator level, not inside the WASM component.

## Provider Operations Reference

| Operation | Description |
|-----------|-------------|
| `render_plan` | Determines rendering tier (AC native, markdown, plain text) |
| `encode` | Serializes `ChannelMessageEnvelope` to base64 payload |
| `send_payload` | Decodes envelope, builds Webex API body, sends POST /messages |
| `send` | Direct send (legacy, builds envelope from flat input) |
| `reply` | Reply in thread (requires `reply_to_id` or `thread_id`) |
| `ingest_http` | Webhook handler: parses event, fetches message, returns `HttpOutV1` |
| `qa-spec` | Returns setup questions for the provider |
| `apply-answers` | Applies QA answers to produce provider config |
| `i18n-keys` | Returns i18n key catalog |
