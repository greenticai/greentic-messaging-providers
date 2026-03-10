# Running the Greentic Demo Operator

This guide covers how to set up and run the Greentic demo operator locally with all messaging providers.

## Prerequisites

| Requirement | How to get |
|-------------|-----------|
| `greentic-operator` binary | `cargo install --path greentic-operator` or build from source |
| Demo bundle | `demo-bundle/` directory with gtpacks + secrets |
| ngrok or cloudflared | For public webhook URL (Telegram, Slack, Teams, Webex) |
| Provider credentials | Bot tokens, API keys (see per-provider sections) |

## Demo Bundle Structure

```
demo-bundle/
├── greentic.demo.yaml          # Bundle marker
├── greentic.toml               # Config (env=dev, port=8080)
├── seeds.yaml                  # Encrypted secrets reference
├── providers/
│   └── messaging/              # Provider gtpacks (8 providers)
│       ├── messaging-telegram.gtpack
│       ├── messaging-slack.gtpack
│       ├── messaging-teams.gtpack
│       ├── messaging-webex.gtpack
│       ├── messaging-webchat.gtpack
│       ├── messaging-email.gtpack
│       ├── messaging-whatsapp.gtpack
│       └── messaging-dummy.gtpack
├── packs/                      # Application packs
│   └── default.gtpack          # Echo bot (Handlebars templating)
├── tenants/
│   └── default/
│       └── tenant.gmap         # Access policy
├── .greentic/
│   └── dev/
│       └── .dev.secrets.env    # Encrypted secrets store
└── state/                      # Runtime state (auto-created)
```

## Quick Start

### 1. Set Environment

```bash
export GREENTIC_ENV=dev
cd /path/to/demo-bundle
```

`GREENTIC_ENV=dev` is required — the secrets backend reads from `.greentic/dev/.dev.secrets.env`.

### 2. Start ngrok (Public Tunnel)

Needed for providers that require webhooks (Telegram, Slack, Teams, Webex).

```bash
# Terminal 1: start ngrok
ngrok http 8080
```

Note the HTTPS URL, e.g. `https://ab12-34-56-78.ngrok-free.app`.

Alternatively, use cloudflared (operator can start it automatically):

```bash
# Operator will start cloudflared if --cloudflared is not set to off
GREENTIC_ENV=dev gtc op demo start --bundle .
```

### 3. Start the Operator

```bash
# Terminal 2: start operator
GREENTIC_ENV=dev gtc op demo start \
  --bundle . \
  --tenant default \
  --verbose
```

Options:
- `--cloudflared off` — disable auto cloudflared tunnel (use ngrok instead)
- `--verbose` — extra logging
- `--tenant default` — which tenant config to load

Expected output:
```
messaging: running embedded runner (no gsm gateway/egress)
events: handled in-process (HTTP ingress + timer scheduler)
secrets: backend=dev-store ...
HTTP ingress ready at http://127.0.0.1:8080
demo start running (bundle=. targets=[default]); press Ctrl+C to stop
```

### 4. Verify

```bash
curl http://localhost:8080/health
```

## Seeding Secrets

Secrets are encrypted in `.greentic/dev/.dev.secrets.env`. Use the `seed_all` tool to batch-write secrets.

**IMPORTANT**: Write ALL secrets for a category in a single session to avoid the DEK cache bug.

```bash
# Build the seed tool
cd /path/to/greentic
cargo build --release -p seed-secret --manifest-path tools/seed-secret/Cargo.toml

# Batch seed example (Telegram)
GREENTIC_ENV=dev ./tools/seed-secret/target/release/seed-secret \
  demo-bundle/.greentic/dev/.dev.secrets.env \
  "secrets://dev/default/_/messaging-telegram/telegram_bot_token" "YOUR_BOT_TOKEN" \
  "secrets://dev/default/_/messaging-telegram/bot_token" "YOUR_BOT_TOKEN"
```

### Secrets per Provider

| Provider | Secret Path | Keys |
|----------|------------|------|
| Telegram | `secrets://dev/default/_/messaging-telegram/` | `telegram_bot_token`, `bot_token` |
| Slack | `secrets://dev/default/_/messaging-slack/` | `slack_bot_token` |
| Teams | `secrets://dev/demo/_/messaging-teams/` | `MS_GRAPH_TENANT_ID`, `MS_GRAPH_CLIENT_ID`, `MS_GRAPH_REFRESH_TOKEN` |
| Webex | `secrets://dev/default/_/messaging-webex/` | `webex_bot_token`, `bot_token` |
| Email | `secrets://dev/default/_/messaging-email/` | `from_address`, `graph_tenant_id`, `ms_graph_client_id`, `ms_graph_refresh_token` |
| WhatsApp | `secrets://dev/default/_/messaging-whatsapp/` | `whatsapp_token`, `phone_number_id` |

## Provider Setup

### Telegram

1. Create a bot via [@BotFather](https://t.me/BotFather) on Telegram
2. Seed the bot token (see above)
3. Start the operator
4. Set the webhook:

```bash
NGROK_URL="https://your-ngrok-url.ngrok-free.app"
BOT_TOKEN="your-bot-token"

# Set webhook
curl "https://api.telegram.org/bot${BOT_TOKEN}/setWebhook?url=${NGROK_URL}/v1/messaging/ingress/messaging-telegram/default/default"

# Verify webhook
curl -s "https://api.telegram.org/bot${BOT_TOKEN}/getWebhookInfo" | jq .
```

5. Test egress:
```bash
GREENTIC_ENV=dev gtc op demo send \
  --bundle . --provider messaging-telegram \
  --to "CHAT_ID" --text "Hello!"
```

6. Test ingress: send a message to the bot on Telegram — the operator receives it via webhook.

### Slack

1. Create a Slack App at [api.slack.com/apps](https://api.slack.com/apps)
2. Add Bot Token Scopes: `chat:write`, `channels:read`, `groups:read`
3. Install app to workspace, copy Bot User OAuth Token (`xoxb-...`)
4. Seed the token
5. Set Event Subscriptions URL: `{NGROK_URL}/v1/messaging/ingress/messaging-slack/default/default`
6. Subscribe to bot events: `message.channels`, `message.groups`, `message.im`
7. Enable Interactivity with the same URL (for button clicks)

App Manifest (JSON) for quick setup:
```json
{
  "display_information": { "name": "Greentic Bot" },
  "features": {
    "bot_user": { "display_name": "Greentic", "always_online": true }
  },
  "oauth_config": {
    "scopes": {
      "bot": ["chat:write", "channels:read", "groups:read", "im:history", "channels:history", "groups:history"]
    }
  },
  "settings": {
    "event_subscriptions": {
      "request_url": "NGROK_URL/v1/messaging/ingress/messaging-slack/default/default",
      "bot_events": ["message.channels", "message.groups", "message.im"]
    },
    "interactivity": {
      "is_enabled": true,
      "request_url": "NGROK_URL/v1/messaging/ingress/messaging-slack/default/default"
    }
  }
}
```

Test egress:
```bash
GREENTIC_ENV=dev gtc op demo send \
  --bundle . --provider messaging-slack \
  --to "C0CHANNEL_ID" --text "Hello Slack!"
```

### Teams

Teams uses Microsoft Graph API for sending and Bot Framework for receiving.

1. Register an Azure AD app (public client, no client_secret)
2. API permissions: `ChannelMessage.Send` (Delegated), `Channel.ReadBasic.All`
3. Get OAuth refresh token via authorization_code flow
4. Seed secrets: `MS_GRAPH_TENANT_ID`, `MS_GRAPH_CLIENT_ID`, `MS_GRAPH_REFRESH_TOKEN`
5. Register an Azure Bot pointing to: `{NGROK_URL}/v1/messaging/ingress/messaging-teams/default/default`
6. Install the bot in your Teams channel

Test egress:
```bash
GREENTIC_ENV=dev gtc op demo send \
  --bundle . --provider messaging-teams \
  --to "TEAM_ID:CHANNEL_ID" --text "Hello Teams!"
```

See `docs/guide-teams-setup.md` for detailed OAuth flow instructions.

### WebChat (Direct Line)

WebChat works out of the box — no external credentials needed. The operator runs a built-in Direct Line server.

Test via curl:
```bash
# Get token
TOKEN=$(curl -s -X POST http://localhost:8080/token | jq -r .token)

# Create conversation
CONV=$(curl -s -X POST http://localhost:8080/v3/directline/conversations \
  -H "Authorization: Bearer $TOKEN")
CONV_ID=$(echo "$CONV" | jq -r .conversationId)
CONV_TOKEN=$(echo "$CONV" | jq -r .token)

# Send message
curl -X POST "http://localhost:8080/v3/directline/conversations/$CONV_ID/activities" \
  -H "Authorization: Bearer $CONV_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"type":"message","text":"hello","from":{"id":"user1"}}'

# Poll for reply (wait 2 seconds)
sleep 2
curl -s "http://localhost:8080/v3/directline/conversations/$CONV_ID/activities" \
  -H "Authorization: Bearer $CONV_TOKEN" | jq .
```

Test with WebChat SPA:
```bash
cd greentic-webchat
npm install && npm run dev
# Open: http://localhost:5173/dev?directline=http://localhost:8080
```

### Webex

1. Create a bot at [developer.webex.com](https://developer.webex.com/my-apps)
2. Copy bot access token
3. Seed token: `webex_bot_token` and `bot_token`
4. Register webhook via Webex API (or operator does it via setup flow)

Test egress (auto-detects roomId vs email):
```bash
GREENTIC_ENV=dev gtc op demo send \
  --bundle . --provider messaging-webex \
  --to "Y2lzY29zcGFyazov..." --text "Hello Webex!"
```

### Email (Microsoft Graph)

1. Azure AD app with Graph API permissions: `Mail.Send` (Delegated)
2. Get refresh token via OAuth
3. Seed: `from_address`, `graph_tenant_id`, `ms_graph_client_id`, `ms_graph_refresh_token`

```bash
GREENTIC_ENV=dev gtc op demo send \
  --bundle . --provider messaging-email \
  --to "recipient@example.com" --text "Hello from Greentic!"
```

## HTTP Endpoints

| Path | Method | Purpose |
|------|--------|---------|
| `/health` | GET | Health check |
| `/v1/messaging/ingress/{provider}/{tenant}/{team}` | POST | Webhook ingress |
| `/token` | POST | Direct Line token |
| `/v3/directline/conversations` | POST | Create DL conversation |
| `/v3/directline/conversations/{id}/activities` | POST/GET | Send/poll messages |

## Useful Commands

```bash
# List available packs
GREENTIC_ENV=dev gtc op demo list-packs --bundle . --domain all

# List flows in a pack
GREENTIC_ENV=dev gtc op demo list-flows --bundle . --pack messaging-telegram

# Test ingress with synthetic webhook
GREENTIC_ENV=dev gtc op demo ingress \
  --bundle . --provider messaging-telegram \
  --body-json '{"update_id":1,"message":{"message_id":1,"from":{"id":123,"is_bot":false,"first_name":"Test"},"chat":{"id":123,"type":"private"},"date":1234567890,"text":"hello"}}'

# View logs
tail -f logs/operator.log
tail -f logs/cloudflared.log

# Bundle health check
GREENTIC_ENV=dev gtc op demo doctor --bundle .
```

## Rebuilding Provider WASMs

When modifying provider source code:

```bash
cd greentic-messaging-providers

# Build all providers
SKIP_WASM_TOOLS_VALIDATION=1 ./tools/build_components.sh

# Build single provider
SKIP_WASM_TOOLS_VALIDATION=1 cargo build --target wasm32-wasip2 --release -p messaging-provider-telegram

# Update gtpack (replace WASM inside zip)
tmpdir=$(mktemp -d)
mkdir -p "$tmpdir/components/messaging-provider-telegram"
cp target/wasm32-wasip2/release/messaging_provider_telegram.wasm \
   "$tmpdir/components/messaging-provider-telegram/component.wasm"
(cd "$tmpdir" && zip -u /path/to/demo-bundle/providers/messaging/messaging-telegram.gtpack \
   components/messaging-provider-telegram/component.wasm)
rm -rf "$tmpdir"

# Restart operator to pick up new pack
```

**IMPORTANT**: Clean `target/wasm32-wasip2/` before rebuilding to avoid stale WASM cache.

## Troubleshooting

| Problem | Solution |
|---------|----------|
| `secrets: backend=dev-store` but secrets missing | Seed secrets with `GREENTIC_ENV=dev` |
| Port 8080 already in use | Kill existing operator: `pkill greentic-operator` |
| Webhook not receiving | Check ngrok is running, URL matches, provider webhook is set |
| WASM stale after rebuild | Clean `target/wasm32-wasip2/` and rebuild |
| DEK cache bug (wrong decryption) | Batch-seed all secrets per category in one session |
| `subscription_id: null` (Teams) | Check `ensure_provider` accepts `messaging-teams` |

See `docs/guide-troubleshooting.md` for detailed troubleshooting.
