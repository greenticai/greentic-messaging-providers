# Microsoft Teams Provider Setup Guide

Set up the Teams messaging provider to send and receive messages in Microsoft Teams channels using the gtc op demo mode.

## Architecture

```
                    EGRESS (send message)
                    ====================
gtc op demo send
    |
    v
[messaging-teams.gtpack]  -- WASM provider component
    |  render_plan -> encode -> send_payload
    |  (acquires OAuth token via refresh_token grant)
    v
Microsoft Graph API  (POST /teams/{id}/channels/{id}/messages)
    |
    v
Message appears in Teams channel


                    INGRESS (receive message)
                    =========================
User sends message in Teams channel
    |
    v
Azure Bot Service  -- POST webhook -->  Operator HTTP gateway (:8080)
                                             |
                                             v
                                        [messaging-ingress-teams.wasm]
                                             |  parse channelIdentity, body
                                             v
                                        ChannelMessageEnvelope (normalized)
```

**Key details:**

- Egress uses Microsoft Graph API v1.0 for sending channel/chat messages
- Ingress uses Azure Bot Service (or Graph subscriptions) for webhook delivery
- Authentication: OAuth 2.0 with refresh tokens (delegated permissions)
- The Azure app must be a PUBLIC client (no client_secret required)
- Token flow: refresh_token grant first, falls back to client_credentials if no refresh token
- Supports Adaptive Cards natively via Graph API attachments

---

## Prerequisites

| Requirement | Notes |
|-------------|-------|
| `greentic-operator` binary | v0.4.24+ installed (`cargo binstall greentic-operator`) |
| `messaging-teams.gtpack` | In `demo-bundle/providers/messaging/` |
| Azure AD app registration | Portal: portal.azure.com |
| Azure Bot Service | For ingress webhooks (optional for egress-only) |
| `seed-secret` or `seed_all` tool | For writing encrypted secrets to dev store |
| Python 3 / ngrok / cloudflared | For public webhook URL (ingress only) |

---

## Step 1: Azure AD App Registration

1. Go to [Azure Portal](https://portal.azure.com) > Azure Active Directory > App registrations > New registration.

2. Register the application:
   - Name: `greentic-teams-bot` (or similar)
   - Supported account types: Single tenant (or multi-tenant if needed)
   - Redirect URI: `http://localhost:3000/oauth/callback/teams` (for local token acquisition)

3. Configure as a **PUBLIC client** (no client secret):
   - Go to Authentication > Advanced settings
   - Set "Allow public client flows" to **Yes**
   - Save

4. Grant API permissions (Delegated):
   - `Channel.ReadBasic.All`
   - `ChannelMessage.Send`
   - `Team.ReadBasic.All`
   - `ChatMessage.Send` (if sending to chats)

5. Note down:
   - **Tenant ID**: Azure AD > Overview > Tenant ID
   - **Client ID**: App registrations > Your app > Application (client) ID

---

## Step 2: Obtain OAuth Refresh Token

The Teams provider uses delegated permissions with a refresh token. You must complete an OAuth authorization_code flow once to obtain the initial refresh token.

### 2a. Build the authorization URL

```
https://login.microsoftonline.com/{TENANT_ID}/oauth2/v2.0/authorize?
  client_id={CLIENT_ID}
  &response_type=code
  &redirect_uri=http://localhost:3000/oauth/callback/teams
  &scope=https://graph.microsoft.com/.default offline_access
  &response_mode=query
```

Replace `{TENANT_ID}` and `{CLIENT_ID}` with your values. Open this URL in a browser.

### 2b. Complete login

Sign in with the user account that has access to the target Teams team/channel (e.g., `ci@YourOrg.onmicrosoft.com`). Grant the requested permissions.

### 2c. Exchange the authorization code

After login, the browser redirects to your redirect URI with a `code` parameter. Exchange it for tokens:

```bash
curl -X POST "https://login.microsoftonline.com/{TENANT_ID}/oauth2/v2.0/token" \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "client_id={CLIENT_ID}" \
  -d "grant_type=authorization_code" \
  -d "code={AUTHORIZATION_CODE}" \
  -d "redirect_uri=http://localhost:3000/oauth/callback/teams" \
  -d "scope=https://graph.microsoft.com/.default offline_access"
```

**Do not include `client_secret`** -- this is a public client.

### 2d. Save the refresh token

The response contains `access_token` and `refresh_token`. Save the `refresh_token` -- you will seed it in the next step.

```json
{
  "access_token": "eyJ0eXAi...",
  "refresh_token": "0.AXYA...",
  "expires_in": 3600,
  "token_type": "Bearer"
}
```

---

## Step 3: Find Your Team and Channel IDs

Use the access token from Step 2 to query Graph API:

```bash
# List your teams
curl -s -H "Authorization: Bearer {ACCESS_TOKEN}" \
  "https://graph.microsoft.com/v1.0/me/joinedTeams" | jq '.value[] | {id, displayName}'

# List channels in a team
curl -s -H "Authorization: Bearer {ACCESS_TOKEN}" \
  "https://graph.microsoft.com/v1.0/teams/{TEAM_ID}/channels" | jq '.value[] | {id, displayName}'
```

Note down the Team ID and Channel ID. The destination format for sending is `team_id:channel_id`.

Example:
```
Team ID:    c3392cbc-2cb0-48e8-9247-504d8defea40
Channel ID: 19:wQzzrth6t3YA-aEdLzt8Pse3kW3Us-nJl9XzN-5NcEE1@thread.tacv2
```

---

## Step 4: Seed Secrets

Secrets are stored under `secrets://dev/demo/_/messaging-teams/` in the encrypted dev store.

### Required secrets

| Secret Key | Description |
|------------|-------------|
| `MS_GRAPH_TENANT_ID` | Azure AD tenant ID |
| `MS_GRAPH_CLIENT_ID` | App registration client ID |
| `MS_GRAPH_REFRESH_TOKEN` | OAuth refresh token from Step 2 |

`MS_GRAPH_CLIENT_SECRET` is **not required** for public client apps. Only seed it if your app uses confidential client credentials.

### Batch seeding (required)

Due to a DEK cache bug in `greentic-secrets-core` v0.4.22, all secrets for a given category must be written in a single session. Writing secrets in separate sessions causes each to get a different data encryption key, making previously written secrets unreadable.

Use the `seed_all` binary or equivalent batch seeder:

```bash
# Seed all Teams secrets in one session
./tools/seed_all/target/release/seed_all \
  demo-bundle/.greentic/dev/.dev.secrets.env \
  "secrets://dev/demo/_/messaging-teams/MS_GRAPH_TENANT_ID={YOUR_TENANT_ID}" \
  "secrets://dev/demo/_/messaging-teams/MS_GRAPH_CLIENT_ID={YOUR_CLIENT_ID}" \
  "secrets://dev/demo/_/messaging-teams/MS_GRAPH_REFRESH_TOKEN={YOUR_REFRESH_TOKEN}"
```

Alternatively, if using the single-secret `seed-secret` tool, run all three in immediate succession within the same process or ensure the store is written atomically. The safest approach is the batch seeder.

### Verify secrets

```bash
GREENTIC_ENV=dev gtc op demo secrets list \
  --bundle demo-bundle --tenant demo
```

---

## Step 5: Verify the gtpack

Confirm the pack file is in the correct location:

```
demo-bundle/
  providers/
    messaging/
      messaging-teams.gtpack
```

You can inspect pack health:

```bash
greentic-pack doctor --validate demo-bundle/providers/messaging/messaging-teams.gtpack
```

---

## Step 6: Start the Operator

```bash
GREENTIC_ENV=dev gtc op demo start \
  --bundle demo-bundle \
  --tenant default \
  --env dev
```

The operator starts an HTTP gateway on `http://127.0.0.1:8080` with:
- Ingress webhook route: `/v1/messaging/ingress/messaging-teams/{tenant}/{team}`
- Embedded NATS (default, no external dependency)

For ingress to work, you need a publicly accessible URL. Use ngrok or cloudflared:

```bash
# In a separate terminal
ngrok http 8080
# or
cloudflared tunnel --url http://localhost:8080
```

Note the public URL for webhook configuration in Step 8.

---

## Step 7: Test Egress (Send a Message)

### Send to a Teams channel

The `--to` flag takes the format `team_id:channel_id`:

```bash
GREENTIC_ENV=dev gtc op demo send \
  --bundle demo-bundle \
  --provider messaging-teams \
  --to "c3392cbc-2cb0-48e8-9247-504d8defea40:19:wQzzrth6t3YA-aEdLzt8Pse3kW3Us-nJl9XzN-5NcEE1@thread.tacv2" \
  --text "Hello from Greentic" \
  --tenant default
```

Expected output on success:

```json
{
  "ok": true,
  "status": "sent",
  "provider_type": "messaging.teams.graph",
  "message_id": "1234567890",
  "provider_message_id": "teams:1234567890"
}
```

### Send to a chat (1:1 or group chat)

```bash
GREENTIC_ENV=dev gtc op demo send \
  --bundle demo-bundle \
  --provider messaging-teams \
  --to "{CHAT_ID}" \
  --text "Hello from Greentic" \
  --tenant default \
  --args-json '{"kind":"chat"}'
```

### Send pipeline breakdown

The operator invokes three WASM operations in sequence:

1. **render_plan** -- Determines rendering tier. Teams supports Adaptive Cards, Markdown, HTML, images, and buttons natively.
2. **encode** -- Serializes the message into a `ProviderPayloadV1` with the full `ChannelMessageEnvelope`. If an Adaptive Card is present in metadata, it injects `_ac_json` for native AC rendering.
3. **send_payload** -- Decodes the payload, acquires an OAuth token via `refresh_token` grant, and POSTs to the Graph API endpoint.

---

## Step 8: Test Ingress (Receive Messages)

### Option A: Simulated ingress via HTTP

Test the ingress pipeline without Azure Bot Service by posting a synthetic webhook payload to the operator gateway:

```bash
curl -X POST http://localhost:8080/v1/messaging/ingress/messaging-teams/default/default \
  -H "Content-Type: application/json" \
  -d '{
    "type": "message",
    "text": "hello",
    "from": {"id": "user1", "name": "Test User"},
    "channelId": "msteams",
    "conversation": {"id": "19:abc@thread.tacv2"},
    "recipient": {"id": "bot1"}
  }'
```

### Option B: Azure Bot Service webhook (production)

For real Teams messages to reach the operator, configure Azure Bot Service:

1. Go to [Azure Portal](https://portal.azure.com) > Bot Services > Your Bot > Settings > Configuration
2. Set the Messaging endpoint to:
   ```
   https://{YOUR_TUNNEL_URL}/v1/messaging/ingress/messaging-teams/default/default
   ```
3. This must be configured manually in the Azure Portal each time the tunnel URL changes.

### Option C: Graph Subscriptions

The Teams pack also supports Graph API change subscriptions for channel messages. The operator can manage these via the `sync_subscriptions` flow, which calls:
- `subscription_ensure` -- Creates or reuses a Graph subscription
- `subscription_renew` -- Extends subscription expiration
- `subscription_delete` -- Removes a subscription

---

## Secret Key Reference

The WASM component resolves configuration from secrets using these keys (case-insensitive lookup):

| Key | Required | Default | Description |
|-----|----------|---------|-------------|
| `MS_GRAPH_TENANT_ID` | Yes | -- | Azure AD tenant identifier |
| `MS_GRAPH_CLIENT_ID` | Yes | -- | Azure AD application client ID |
| `MS_GRAPH_REFRESH_TOKEN` | Yes* | -- | OAuth2 refresh token (delegated flow) |
| `MS_GRAPH_CLIENT_SECRET` | No | -- | Only for confidential client apps |

*If no refresh token is available, the component falls back to `client_credentials` grant, which requires `MS_GRAPH_CLIENT_SECRET`.

### Provider config fields (via setup or direct config)

| Field | Required | Default |
|-------|----------|---------|
| `tenant_id` | Yes | -- |
| `client_id` | Yes | -- |
| `public_base_url` | Yes | -- |
| `team_id` | No | -- |
| `channel_id` | No | -- |
| `graph_base_url` | No | `https://graph.microsoft.com/v1.0` |
| `auth_base_url` | No | `https://login.microsoftonline.com` |
| `token_scope` | No | `https://graph.microsoft.com/.default` |
| `client_secret` | No | -- |
| `refresh_token` | No | -- |

---

## Known Issues

### Public client constraint
The Azure app is configured as a PUBLIC client. Do not send `client_secret` with the `refresh_token` grant -- the token endpoint will reject it. If you see `AADSTS7000218` errors, verify that "Allow public client flows" is enabled in your app registration.

### Refresh token expiration
Refresh tokens have a limited lifetime (typically 90 days for single-tenant apps). When expired, you must repeat Step 2 to obtain a new one and re-seed the `MS_GRAPH_REFRESH_TOKEN` secret.

### Webhook URL requires manual configuration
Azure Bot Service does not support programmatic webhook URL updates through the operator. You must update the messaging endpoint in the Azure Portal whenever the tunnel URL changes.

### DEK cache bug
`greentic-secrets-core` v0.4.22 shares a single DEK cache slot per `(env, tenant, team, category)`. Writing secrets in separate sessions causes each to get a different data encryption key. Always batch-seed all secrets for the `messaging-teams` category in a single session.

---

## Troubleshooting

| Symptom | Likely Cause | Fix |
|---------|-------------|-----|
| `401 Unauthorized` from Graph API | Expired refresh token | Re-authenticate (Step 2) and re-seed the token |
| `403 Forbidden` from Graph API | Missing API permissions | Add required delegated permissions in Azure Portal |
| `AADSTS7000218` | client_secret sent for public client | Ensure app is configured as public client, no secret in secrets store |
| `missing secret: MS_GRAPH_TENANT_ID` | Secrets not seeded or DEK mismatch | Re-seed all secrets in a single batch session |
| `destination must be team_id:channel_id` | Wrong `--to` format | Use `"team_id:channel_id"` with both values separated by colon |
| `provider type mismatch` | Wrong provider name in `--provider` flag | Use `--provider messaging-teams` (matches gtpack name) |
| `token endpoint returned status 400` | Malformed token request | Verify tenant_id and client_id are correct |
| Ingress returns empty/error | HttpOutV1 version mismatch | Ensure operator version matches WASM component version |

### Debug commands

```bash
# Validate pack health
greentic-pack doctor --validate demo-bundle/providers/messaging/messaging-teams.gtpack

# List available flows
GREENTIC_ENV=dev gtc op demo list-flows \
  --bundle demo-bundle --pack messaging-teams --domain messaging

# Run diagnostics flow
GREENTIC_ENV=dev gtc op demo run-flow \
  --bundle demo-bundle --pack messaging-teams --flow diagnostics --tenant default
```

---

## Supported Operations

The `messaging-provider-teams` WASM component exposes these operations:

| Operation | Description |
|-----------|-------------|
| `send` | Send a message to a Teams channel or chat |
| `reply` | Reply in a Teams thread (requires `reply_to_id`) |
| `ingest_http` | Normalize an incoming Teams webhook payload into `ChannelMessageEnvelope` |
| `render_plan` | Determine rendering tier (AC/markdown/html/plain) |
| `encode` | Encode a universal message into a Teams-specific `ProviderPayloadV1` |
| `send_payload` | Execute the encoded payload against Graph API |
| `subscription_ensure` | Create or reuse a Graph change subscription |
| `subscription_renew` | Extend a subscription's expiration |
| `subscription_delete` | Remove a Graph subscription |
