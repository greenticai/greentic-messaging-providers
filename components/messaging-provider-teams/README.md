# Messaging Provider Teams Component

Provider-core Microsoft Teams messaging provider (Graph API send).

## Component ID
- `messaging-provider-teams`

## Provider types
- `messaging.teams.graph`

## Secrets

All secrets are stored under the URI prefix
`secrets://{env}/{tenant}/_/messaging-teams/` where `{env}` must match
`GREENTIC_ENV` (e.g. `dev`).

| Key | Required | Description |
|-----|----------|-------------|
| `MS_GRAPH_TENANT_ID` | Yes | Azure AD tenant ID |
| `MS_GRAPH_CLIENT_ID` | Yes | Azure AD application (client) ID |
| `MS_GRAPH_CLIENT_SECRET` | Confidential clients only | Client secret for `client_credentials` grant |
| `MS_GRAPH_REFRESH_TOKEN` | Public clients only | Refresh token for delegated `authorization_code` grant |

### Public vs Confidential client

- **Public client** (e.g. mobile/desktop app registration): Uses `refresh_token` grant.
  The Azure app must **not** have a client secret configured, and the refresh
  request must **not** include `client_secret`. Only `tenant_id`, `client_id`,
  and `refresh_token` are needed.
- **Confidential client** (e.g. web app with secret): Uses `client_credentials`
  grant. Requires `tenant_id`, `client_id`, and `client_secret`. No refresh
  token is needed.

### Obtaining a refresh token (public client)

1. Register an app in Azure AD with **Delegated** permissions:
   `ChannelMessage.Send`, `Chat.ReadWrite`, `offline_access`.
2. Set the redirect URI to `http://localhost` and enable **public client flows**.
3. Open this URL in a browser (replace placeholders):
   ```
   https://login.microsoftonline.com/{tenant_id}/oauth2/v2.0/authorize?client_id={client_id}&response_type=code&redirect_uri=http://localhost&scope=ChannelMessage.Send+Chat.ReadWrite+offline_access
   ```
4. Sign in and copy the `code` parameter from the redirect URL.
5. Exchange the code for tokens:
   ```bash
   curl -X POST "https://login.microsoftonline.com/{tenant_id}/oauth2/v2.0/token" \
     -d "client_id={client_id}&grant_type=authorization_code&code={code}&redirect_uri=http://localhost&scope=ChannelMessage.Send+Chat.ReadWrite+offline_access"
   ```
6. Save the `refresh_token` from the response as the `MS_GRAPH_REFRESH_TOKEN` secret.

## Destination formats

The `--to` flag (or `to[0].id` in the envelope) accepts two formats:

| Format | Kind | Example |
|--------|------|---------|
| `{team_id}:{channel_id}` | `channel` | `c3392cbc-2cb0-48e8-9247-504d8defea40:19:abc...@thread.tacv2` |
| `{chat_id}` | `chat` | `19:meeting_abc...@thread.v2` |

The provider auto-detects the kind based on whether the ID contains a `:` separator.

## Quick start

### Send a text message to a channel

```bash
GREENTIC_ENV=dev greentic-operator demo send \
  --bundle demo-bundle \
  --provider messaging-teams \
  --to "{team_id}:{channel_id}" \
  --text "Hello from Greentic" \
  --tenant demo --env dev
```

### Test ingress (CLI)

```bash
GREENTIC_ENV=dev greentic-operator demo ingress \
  --bundle demo-bundle \
  --provider messaging-teams \
  --tenant demo \
  --body /tmp/teams-webhook.json
```

### Test ingress (operator HTTP)

```bash
# Start operator
GREENTIC_ENV=dev greentic-operator demo start \
  --bundle demo-bundle --cloudflared off --nats off \
  --skip-setup --skip-secrets-init --domains messaging

# POST webhook
curl -X POST http://localhost:8080/messaging/ingress/messaging-teams/demo/default \
  -H "Content-Type: application/json" \
  -d @/tmp/teams-webhook.json
```
