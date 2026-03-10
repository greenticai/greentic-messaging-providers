# WebChat Provider Setup Guide

Set up the WebChat messaging provider with `greentic-operator` in demo mode. WebChat uses the Microsoft Direct Line v3 protocol with a host-backed state-store -- no external webhooks or third-party accounts required.

---

## Architecture

```
Browser (WebChat SPA)
    |  REST polling + WebSocket
    v
greentic-operator (HTTP :8080)
    |  /token           -- JWT issuance (HMAC-SHA256)
    |  /v3/directline/* -- Direct Line v3 endpoints
    v
messaging-webchat.gtpack (WASM)
    |  WIT state-store interface
    v
Host state-store (in-memory)
```

Key difference from other providers: WebChat does not call any external API. The operator itself acts as the Direct Line server. Conversations are stored in the host state-store and persist across WASM invocations.

---

## Prerequisites

| Requirement | Notes |
|-------------|-------|
| `greentic-operator` binary | Installed at `~/.cargo/bin/greentic-operator` or built from source |
| `messaging-webchat.gtpack` | In `demo-bundle/providers/messaging/` |
| `jq` | For manual testing with curl |
| Node.js 18+ | Only needed if running the WebChat SPA dev server |
| Existing demo bundle | `demo-bundle/greentic.demo.yaml` must exist |

---

## Step 1: Verify the gtpack is in the bundle

```bash
ls demo-bundle/providers/messaging/messaging-webchat.gtpack
```

If missing, copy it from the providers build output:

```bash
cp greentic-messaging-providers/dist/packs/messaging-webchat.gtpack \
   demo-bundle/providers/messaging/
```

---

## Step 2: Start the operator

```bash
GREENTIC_ENV=dev gtc op demo start \
  --bundle demo-bundle \
  --tenant default \
  --env dev
```

The operator starts an HTTP server on port 8080 with these endpoints:

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/token` | POST | Issue a JWT token |
| `/v3/directline/conversations` | POST | Create a new conversation |
| `/v3/directline/conversations/{id}/activities` | POST | Send a message |
| `/v3/directline/conversations/{id}/activities` | GET | Poll for messages |

Leave this running. All subsequent commands run in a separate terminal.

---

## Step 3: Test the Direct Line protocol manually

This verifies the full round-trip without a browser.

### 3a. Get a token

```bash
TOKEN=$(curl -s -X POST http://localhost:8080/token | jq -r .token)
echo "$TOKEN"
```

### 3b. Create a conversation

```bash
CONV=$(curl -s -X POST http://localhost:8080/v3/directline/conversations \
  -H "Authorization: Bearer $TOKEN")
CONV_ID=$(echo "$CONV" | jq -r .conversationId)
CONV_TOKEN=$(echo "$CONV" | jq -r .token)

echo "Conversation: $CONV_ID"
```

**Important:** The POST `/conversations` response returns a conversation-bound token in the `.token` field. Use `$CONV_TOKEN` (not the original `$TOKEN`) for all subsequent requests on this conversation.

### 3c. Send a message

```bash
curl -s -X POST \
  "http://localhost:8080/v3/directline/conversations/$CONV_ID/activities" \
  -H "Authorization: Bearer $CONV_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"type":"message","text":"hello","from":{"id":"user1"}}'
```

### 3d. Poll for the reply

Wait a couple of seconds for the echo bot to process, then retrieve activities:

```bash
sleep 2
curl -s "http://localhost:8080/v3/directline/conversations/$CONV_ID/activities" \
  -H "Authorization: Bearer $CONV_TOKEN" | jq .
```

You should see both your original message and the bot's reply in the `activities` array.

### All-in-one script

```bash
#!/usr/bin/env bash
set -euo pipefail

BASE="http://localhost:8080"

TOKEN=$(curl -s -X POST "$BASE/token" | jq -r .token)
CONV=$(curl -s -X POST "$BASE/v3/directline/conversations" \
  -H "Authorization: Bearer $TOKEN")
CONV_ID=$(echo "$CONV" | jq -r .conversationId)
CONV_TOKEN=$(echo "$CONV" | jq -r .token)

echo "Conversation: $CONV_ID"

curl -s -X POST "$BASE/v3/directline/conversations/$CONV_ID/activities" \
  -H "Authorization: Bearer $CONV_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"type":"message","text":"hello from curl","from":{"id":"user1"}}' | jq .

sleep 2

echo "--- Activities ---"
curl -s "$BASE/v3/directline/conversations/$CONV_ID/activities" \
  -H "Authorization: Bearer $CONV_TOKEN" | jq .activities
```

---

## Step 4: WebChat SPA (browser)

The `greentic-webchat` repo contains a React SPA that wraps Microsoft BotFramework-WebChat and connects via Direct Line.

### 4a. Install dependencies

```bash
cd greentic-webchat
npm install
```

### 4b. Start the dev server

```bash
npm run dev
```

Vite starts on `http://localhost:5173`.

### 4c. Open in browser

```
http://localhost:5173/dev?directline=http://localhost:8080
```

URL breakdown:

| Part | Meaning |
|------|---------|
| `/dev` | Loads the `dev` tenant skin from `public/skins/dev/skin.json` |
| `?directline=http://localhost:8080` | Overrides the Direct Line domain to your local operator |

The `dev` skin is pre-configured with `tokenUrl: "http://localhost:8080/token"`, so the `?directline=` parameter provides the domain for the `/v3/directline/*` API calls while the skin's `tokenUrl` handles token acquisition.

### How the SPA resolves Direct Line

1. The URL path's first segment selects the tenant (`dev`).
2. The SPA fetches `public/skins/dev/skin.json` to get the `directLine.tokenUrl`.
3. If `?directline=` is set, the SPA tries to use that URL as the token endpoint first. If it fails (because the URL is a domain, not a `/token` endpoint), it falls back to the skin's `tokenUrl` and uses the `?directline=` value as the Direct Line domain.
4. The BotFramework-WebChat component connects using the resolved token and domain.

---

## Step 5: Send outbound messages via CLI

To push a message from the platform into an existing WebChat conversation:

```bash
GREENTIC_ENV=dev gtc op demo send \
  --bundle demo-bundle \
  --provider messaging-webchat \
  --to "$CONV_ID" \
  --text "Hello from Greentic" \
  --tenant default
```

Replace `$CONV_ID` with an active conversation ID from Step 3b or from the SPA's network inspector.

---

## Troubleshooting

### Token errors (401 Unauthorized)

**Symptom:** Requests to `/v3/directline/conversations/{id}/activities` return 401.

**Cause:** Using the initial token from `POST /token` instead of the conversation-bound token from `POST /v3/directline/conversations`.

**Fix:** Always extract and use the `.token` field from the `POST /conversations` response for subsequent calls on that conversation.

### CORS errors in browser

The operator returns `access-control-allow-origin: *` on all Direct Line endpoints. If you still see CORS errors, verify the operator is running and reachable at the URL specified in `?directline=`.

### SPA shows "Unable to load skin" error

The URL path must match a skin directory under `public/skins/`. For local development, use `/dev` which maps to `public/skins/dev/skin.json`.

### Conversation state not persisting

WebChat uses the WIT `state-store` interface backed by the operator's host memory. Conversations survive across WASM invocations within a single operator session, but are lost when the operator restarts.

### JWT signing key

The signing key is auto-generated at operator startup if not seeded in secrets. The secret category is `messaging-webchat` (with hyphen). For persistent tokens across restarts, seed the key:

```bash
# Seed signing key into dev secrets store (optional)
GREENTIC_ENV=dev gtc op demo seed-secret \
  --bundle demo-bundle \
  --tenant default \
  --category messaging-webchat \
  --key jwt_signing_key \
  --value "your-base64-encoded-key"
```

### Operator not starting

Verify the gtpack exists and the bundle structure is correct:

```
demo-bundle/
  greentic.demo.yaml
  providers/
    messaging/
      messaging-webchat.gtpack
```

---

## Reference

### Direct Line v3 API (as implemented by operator)

**POST /token**

Returns a short-lived JWT for initiating conversations.

```json
{ "token": "eyJhbG..." }
```

**POST /v3/directline/conversations**

Request header: `Authorization: Bearer <token>`

Response:
```json
{
  "conversationId": "conv-abc123",
  "token": "eyJhbG...",
  "expires_in": 1800
}
```

**POST /v3/directline/conversations/{id}/activities**

Request header: `Authorization: Bearer <conversation-token>`

Request body:
```json
{
  "type": "message",
  "text": "hello",
  "from": { "id": "user1" }
}
```

Response:
```json
{ "id": "activity-xyz" }
```

**GET /v3/directline/conversations/{id}/activities**

Request header: `Authorization: Bearer <conversation-token>`

Response:
```json
{
  "activities": [
    { "id": "1", "type": "message", "text": "hello", "from": { "id": "user1" } },
    { "id": "2", "type": "message", "text": "Echo: hello", "from": { "id": "bot" } }
  ],
  "watermark": "2"
}
```

### Skin configuration (dev)

File: `greentic-webchat/apps/webchat-spa/public/skins/dev/skin.json`

```json
{
  "tenant": "dev",
  "mode": "fullpage",
  "directLine": {
    "tokenUrl": "http://localhost:8080/token"
  }
}
```

### Related documentation

- [Running Guide](/docs/04-running-guide.md) -- full operator lifecycle (build, setup, start, test)
- [Local Development Guide](/docs/12-local-development.md) -- one-time setup and bundle creation
- [Messaging Providers Deep Dive](/docs/06-messaging-providers-deep-dive.md) -- provider architecture
