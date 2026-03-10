# Cisco RFQ Demo Guide — WebChat + Teams + Webex

Run the Cisco RFQ Compliance demo with interactive Adaptive Cards across three platforms: WebChat (browser), Microsoft Teams, and Webex.

## Architecture

```
                         cisco-bundle.gtpack (app pack)
                          14 Adaptive Card nodes (AC v1.3)
                          RFQ compliance flow engine
                                    │
                    ┌───────────────┼───────────────┐
                    ▼               ▼               ▼
             messaging-webchat  messaging-teams  messaging-webex
              (Direct Line)     (Graph API)      (Webex API)
                    │               │               │
                    ▼               ▼               ▼
               WebChat SPA     Teams Channel     Webex Space
               (browser)       (desktop/web)     (desktop/web)
```

The same card JSON (AC v1.3) is used on all three platforms. Each provider handles delivery natively.

---

## Prerequisites

| Requirement | Notes |
|-------------|-------|
| `greentic-operator` | v0.4.30+ at `/root/.cargo/bin/greentic-operator` |
| `demo-bundle/` | With all provider gtpacks and seeded secrets |
| `cisco-bundle.gtpack` | Deployed as `demo-bundle/packs/default.gtpack` |
| `cloudflared` | For public webhook URL (Teams/Webex ingress) |
| Seeded secrets | Teams: `demo` tenant, Webex: `default` tenant |

### Verify prerequisites

```bash
export GREENTIC_ENV=dev
export BUNDLE=/root/works/personal/greentic/demo-bundle

# Operator binary
which greentic-operator
# Expected: /root/.cargo/bin/greentic-operator

# Provider packs
ls $BUNDLE/providers/messaging/messaging-{teams,webex,webchat}.gtpack

# App pack (cisco bundle)
ls -la $BUNDLE/packs/default.gtpack
# Expected: ~5.5 MB

# Secrets store
ls -la $BUNDLE/.greentic/dev/.dev.secrets.env
```

---

## Part 1: Egress Only (Send Cards Without Ingress)

No server needed. Send individual cards to any platform.

### 1a. Send to Teams

```bash
GREENTIC_ENV=dev gtc op demo send \
  --bundle demo-bundle \
  --provider messaging-teams \
  --tenant demo --env dev \
  --to "c3392cbc-2cb0-48e8-9247-504d8defea40:19:wQzzrth6t3YA-aEdLzt8Pse3kW3Us-nJl9XzN-5NcEE1@thread.tacv2" \
  --text "RFQ Compliance Demo" \
  --card /tmp/cisco/cisco-bundle/cisco-bundle/assets/cards/RFQ-CARD-01_intake.json
```

### 1b. Send to Webex

```bash
GREENTIC_ENV=dev gtc op demo send \
  --bundle demo-bundle \
  --provider messaging-webex \
  --tenant default --env dev \
  --to "Y2lzY29zcGFyazovL3VybjpURUFNOnVzLXdlc3QtMl9yL1JPT00vODRmNTA2NjAtMGRkZC0xMWYxLWI4MGYtYWQ2N2Y3OTk5NDlk" \
  --text "RFQ Compliance Demo" \
  --card /tmp/cisco/cisco-bundle/cisco-bundle/assets/cards/RFQ-CARD-01_intake.json
```

### 1c. Send all 14 RFQ cards (batch)

```bash
cd /root/works/personal/greentic

# Teams — all 14 cards
for card in RFQ-CARD-{01_intake,02_processing,03_tasks_dashboard,04_task_detail,05B_rag_answer,05_rag_edit,06_pack_ready,07_send_review,08_review_dashboard,09_apply_change,10B_send_approval,10_final_approval,11_compliance_summary,STATUS}; do
  GREENTIC_ENV=dev gtc op demo send \
    --bundle demo-bundle --provider messaging-teams \
    --tenant demo --env dev \
    --to "c3392cbc-2cb0-48e8-9247-504d8defea40:19:wQzzrth6t3YA-aEdLzt8Pse3kW3Us-nJl9XzN-5NcEE1@thread.tacv2" \
    --text "$card" \
    --card "/tmp/cisco/cisco-bundle/cisco-bundle/assets/cards/${card}.json" 2>&1 | grep '"ok"'
done

# Webex — all 14 cards
for card in RFQ-CARD-{01_intake,02_processing,03_tasks_dashboard,04_task_detail,05B_rag_answer,05_rag_edit,06_pack_ready,07_send_review,08_review_dashboard,09_apply_change,10B_send_approval,10_final_approval,11_compliance_summary,STATUS}; do
  GREENTIC_ENV=dev gtc op demo send \
    --bundle demo-bundle --provider messaging-webex \
    --tenant default --env dev \
    --to "Y2lzY29zcGFyazovL3VybjpURUFNOnVzLXdlc3QtMl9yL1JPT00vODRmNTA2NjAtMGRkZC0xMWYxLWI4MGYtYWQ2N2Y3OTk5NDlk" \
    --text "$card" \
    --card "/tmp/cisco/cisco-bundle/cisco-bundle/assets/cards/${card}.json" 2>&1 | grep '"ok"'
done
```

### Provider destination reference

| Provider | `--provider` | `--tenant` | `--to` format |
|----------|-------------|-----------|---------------|
| Teams | `messaging-teams` | `demo` | `team_id:channel_id` |
| Webex | `messaging-webex` | `default` | Room ID (`Y2lz...`) |
| WebChat | `messaging-webchat` | `default` | (via Direct Line, not `demo send`) |

---

## Part 2: Full Interactive Flow (Ingress + Egress)

For the interactive card flow (user clicks button → next card appears), you need:
1. Operator running as HTTP server
2. Public URL via cloudflared tunnel (for Teams/Webex webhooks)
3. Webhook registered on each platform

### Step 1: Start the operator

```bash
cd /root/works/personal/greentic

GREENTIC_ENV=dev gtc op demo start \
  --bundle demo-bundle \
  --cloudflared on \
  --skip-setup \
  --skip-secrets-init \
  --domains messaging
```

This starts:
- HTTP gateway at `http://127.0.0.1:8080`
- Cloudflared tunnel → public URL (e.g. `https://xxxx.trycloudflare.com`)
- Direct Line server for WebChat at `/v3/directline/*`

### Step 2: Get the public URL

```bash
# From cloudflared logs
TUNNEL=$(grep -oP 'https://[a-z0-9-]+\.trycloudflare\.com' demo-bundle/logs/cloudflared.log | tail -1)
echo "Public URL: $TUNNEL"
```

Or check `demo-bundle/state/runtime/public_base_url.txt`.

### Step 3: Start WebChat SPA (browser)

```bash
cd /root/works/personal/greentic/greentic-webchat/apps/webchat-spa
npm run dev
```

Open `http://localhost:5173/dev` — the WebChat widget connects to `localhost:8080` via Direct Line. Type a message and the cisco RFQ flow responds with interactive Adaptive Cards.

---

## Part 3: Teams Ingress Setup

### 3a. Azure Bot Service configuration

1. Go to [Azure Portal](https://portal.azure.com) → Bot Services → your bot
2. Under **Settings → Configuration**, set the Messaging endpoint:
   ```
   https://<TUNNEL_URL>/v1/messaging/ingress/messaging-teams/demo/default
   ```
3. Save

The endpoint format is:
```
{public_url}/v1/messaging/ingress/{provider}/{tenant}/{team}
```

### 3b. Test ingress

**Simulated (no Azure Bot needed):**

```bash
curl -X POST http://localhost:8080/v1/messaging/ingress/messaging-teams/demo/default \
  -H "Content-Type: application/json" \
  -d '{
    "type": "message",
    "text": "start compliance",
    "from": {"id": "user1", "name": "Test User"},
    "channelId": "msteams",
    "conversation": {"id": "19:abc@thread.tacv2"},
    "recipient": {"id": "bot1"}
  }'
```

**Live (from Teams):**

After setting the messaging endpoint in Azure Portal, send a message to the bot in a Teams channel. The operator receives the webhook and runs the RFQ flow.

### 3c. Card action handling

When a user clicks a button on an Adaptive Card in Teams, Teams sends an `invoke` activity with the `Action.Submit` data payload. The operator receives this at the same webhook endpoint and routes it through the flow engine.

The card data payloads contain routing info:
```json
{
  "flow": "rfq",
  "cardId": "RFQ-CARD-01",
  "step": "startScan",
  "routeToCardId": "RFQ-CARD-02_processing"
}
```

---

## Part 4: Webex Ingress Setup

### 4a. Register webhooks

Webex requires two webhooks — one for text messages and one for card button clicks.

```bash
WEBEX_TOKEN="<your-bot-token>"
TUNNEL="<your-cloudflared-url>"

# Webhook 1: Text messages
curl -X POST "https://webexapis.com/v1/webhooks" \
  -H "Authorization: Bearer $WEBEX_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"name\": \"greentic-messages\",
    \"targetUrl\": \"${TUNNEL}/v1/messaging/ingress/messaging-webex/default/default\",
    \"resource\": \"messages\",
    \"event\": \"created\"
  }"

# Webhook 2: Card actions (user clicks AC button)
curl -X POST "https://webexapis.com/v1/webhooks" \
  -H "Authorization: Bearer $WEBEX_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"name\": \"greentic-card-actions\",
    \"targetUrl\": \"${TUNNEL}/v1/messaging/ingress/messaging-webex/default/default\",
    \"resource\": \"attachmentActions\",
    \"event\": \"created\"
  }"
```

### 4b. Verify webhooks

```bash
curl -s "https://webexapis.com/v1/webhooks" \
  -H "Authorization: Bearer $WEBEX_TOKEN" | python3 -m json.tool
```

Both webhooks should show `"status": "active"`.

### 4c. Update webhooks (after tunnel URL changes)

Cloudflared generates a new URL on every restart. Update existing webhooks:

```bash
# List webhooks to get IDs
curl -s "https://webexapis.com/v1/webhooks" \
  -H "Authorization: Bearer $WEBEX_TOKEN" \
  | python3 -c "import json,sys; [print(f'{w[\"id\"]}  {w[\"name\"]}  {w[\"status\"]}') for w in json.load(sys.stdin)['items']]"

# Update each webhook
WEBHOOK_ID="<id>"
curl -X PUT "https://webexapis.com/v1/webhooks/${WEBHOOK_ID}" \
  -H "Authorization: Bearer $WEBEX_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"name\": \"greentic-messages\",
    \"targetUrl\": \"${TUNNEL}/v1/messaging/ingress/messaging-webex/default/default\"
  }"
```

### 4d. Test ingress

**Simulated:**

```bash
curl -X POST http://localhost:8080/v1/messaging/ingress/messaging-webex/default/default \
  -H "Content-Type: application/json" \
  -d '{
    "resource": "messages",
    "event": "created",
    "data": {
      "id": "Y2lzY29zcGFyazovL3VzL01FU1NBR0UvMTIzNDU2",
      "roomId": "Y2lzY29zcGFyazovL3VybjpURUFNOnVzLXdlc3QtMl9yL1JPT00vODRmNTA2NjAtMGRkZS0xMWYxLWI4MGYtYWQ2N2Y3OTk5NDlk",
      "personId": "Y2lzY29zcGFyazovL3VzL1BFT1BMRS91c2VyMTIz",
      "personEmail": "user@example.com",
      "created": "2026-02-27T00:00:00.000Z"
    }
  }'
```

**Live:** Send a message to the bot in a Webex Space. The webhook fires and the operator processes it.

---

## Part 5: RFQ Card Flow Sequence

The Cisco RFQ flow has 14 interactive Adaptive Cards:

```
RFQ-CARD-01  (Intake)
    │ User clicks "Start compliance scan"
    ▼
RFQ-CARD-02  (Processing)
    │ Auto-advance or user clicks "View tasks"
    ▼
RFQ-CARD-03  (Tasks Dashboard — 12 Pass, 3 Clarify, 1 Gap)
    │ User clicks a task row
    ▼
RFQ-CARD-04  (Task Detail)
    │ User clicks "Ask AI"
    ▼
RFQ-CARD-05B (RAG Answer)
    │ User edits or accepts
    ▼
RFQ-CARD-05  (RAG Edit)
    │ User saves
    ▼
RFQ-CARD-06  (Pack Ready)
    │ User clicks "Send for review"
    ▼
RFQ-CARD-07  (Send Review)
    │ User sends
    ▼
RFQ-CARD-08  (Review Dashboard)
    │ Reviewer applies changes
    ▼
RFQ-CARD-09  (Apply Change)
    │ User clicks "Final approval"
    ▼
RFQ-CARD-10  (Final Approval)
    │ User approves
    ▼
RFQ-CARD-10B (Send Approval notification)
    │
    ▼
RFQ-CARD-11  (Compliance Summary)

RFQ-CARD-STATUS  (Status dashboard — accessible from any card)
```

Each card's `Action.Submit` data contains routing info:
```json
{
  "flow": "rfq",
  "cardId": "RFQ-CARD-01",
  "routeToCardId": "RFQ-CARD-02_processing"
}
```

The flow engine reads `routeToCardId` and renders the next card.

---

## Secrets Reference

### Teams (tenant: `demo`)

| Secret | URI |
|--------|-----|
| Tenant ID | `secrets://dev/demo/_/messaging-teams/MS_GRAPH_TENANT_ID` |
| Client ID | `secrets://dev/demo/_/messaging-teams/MS_GRAPH_CLIENT_ID` |
| Refresh Token | `secrets://dev/demo/_/messaging-teams/MS_GRAPH_REFRESH_TOKEN` |

### Webex (tenant: `default`)

| Secret | URI |
|--------|-----|
| Bot Token | `secrets://dev/default/_/messaging-webex/webex_bot_token` |
| Bot Token (compat) | `secrets://dev/default/_/messaging-webex/bot_token` |

### WebChat (tenant: `default`)

WebChat uses state-store (no external secrets needed). Direct Line tokens are generated by the operator.

---

## Webhook URL Reference

| Platform | Ingress URL |
|----------|-------------|
| Teams | `{TUNNEL}/v1/messaging/ingress/messaging-teams/demo/default` |
| Webex (messages) | `{TUNNEL}/v1/messaging/ingress/messaging-webex/default/default` |
| Webex (card actions) | Same URL, resource=`attachmentActions` |
| WebChat | Direct Line at `{HOST}/v3/directline/*` (no webhook needed) |

---

## Troubleshooting

### Cloudflared URL changes every restart

This is expected. After each `demo start`, update:
- Teams: Azure Bot Service → Messaging endpoint
- Webex: `PUT /v1/webhooks/{id}` with new targetUrl

### Webex webhook shows "disabled"

Webex disables webhooks when the target URL is unreachable. Update with the new tunnel URL.

### Webex rejects card with "Invalid URL"

All URLs in Adaptive Card JSON must be absolute (`https://...`). Relative URLs like `/page.html` are rejected by Webex's server-side validation. The cards in the cisco-bundle have been fixed to use `https://demo.greentic.ai/...` as base URL.

### Teams 401 Unauthorized

The OAuth refresh token has expired (90 days). Re-authenticate:

```bash
# 1. Build auth URL
echo "https://login.microsoftonline.com/${TENANT_ID}/oauth2/v2.0/authorize?client_id=${CLIENT_ID}&response_type=code&redirect_uri=http://localhost:3000/oauth/callback/teams&scope=https://graph.microsoft.com/.default%20offline_access&response_mode=query"

# 2. Open URL in browser, sign in, get code from redirect
# 3. Exchange code for tokens
curl -X POST "https://login.microsoftonline.com/${TENANT_ID}/oauth2/v2.0/token" \
  -d "client_id=${CLIENT_ID}&grant_type=authorization_code&code=${CODE}&redirect_uri=http://localhost:3000/oauth/callback/teams&scope=https://graph.microsoft.com/.default offline_access"

# 4. Re-seed refresh_token
```

### Secrets not found

Check that `--tenant` matches the tenant where secrets are seeded:
- Teams uses `--tenant demo`
- Webex uses `--tenant default`

### WebChat 429 Too Many Requests

The `/token` endpoint is being hit too often (hot reload in dev). Wait a few seconds and refresh the page.

---

## Rebuilding the cisco-bundle.gtpack

If you modify card JSON files:

```bash
cd /tmp/cisco/cisco-bundle/cisco-bundle

# Update cards inside the zip
tmpdir=$(mktemp -d)
for card in assets/cards/*.json; do
  mkdir -p "$tmpdir/$(dirname "$card")"
  cp "$card" "$tmpdir/$card"
done
(cd "$tmpdir" && zip -u dist/cisco-bundle.gtpack assets/cards/*.json)
rm -rf "$tmpdir"

# Deploy to demo-bundle
cp dist/cisco-bundle.gtpack /root/works/personal/greentic/demo-bundle/packs/default.gtpack
```

---

## E2E Test Results

| Platform | Cards Sent | Success Rate | AC Rendering |
|----------|-----------|:---:|:---:|
| WebChat | 14/14 | 100% | Native (v1.6 renderer) |
| Teams | 14/14 | 100% | Native (TierA, v1.5 renderer) |
| Webex | 14/14 | 100% | Attachment (TierB, v1.3 renderer) |

Tested: February 27, 2026
