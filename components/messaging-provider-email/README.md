# Messaging Provider Email Component

WASM component for sending emails via Microsoft Graph API (primary) or SMTP (fallback).

## Component ID
- `messaging-provider-email`

## Provider Type
- `messaging.email.smtp`

## How It Works

The email provider sends mail through the Microsoft Graph `/me/sendMail` endpoint using a delegated OAuth 2.0 token. The refresh token is stored in `greentic-secrets` and exchanged at send-time for an access token with `Mail.Send` scope.

```
send_payload
  ├── get refresh_token from secrets store
  ├── exchange for access_token (Graph token endpoint)
  └── POST /v1.0/me/sendMail (Microsoft Graph API)
```

If no refresh token is available but `ms_graph_client_secret` is seeded, it falls back to client_credentials grant (app-only token). This requires the Azure AD app to have `Mail.Send` Application permission with admin consent.

## Secrets

| Key | Required | Description |
|-----|:---:|-------------|
| `from_address` | Yes | Sender email (e.g. `ci@GreenticAI.onmicrosoft.com`) |
| `graph_tenant_id` | Yes | Azure AD tenant ID |
| `ms_graph_client_id` | Yes | Azure AD app (client) ID |
| `ms_graph_refresh_token` | Yes* | OAuth2 refresh token with `Mail.Send` delegated scope |
| `ms_graph_client_secret` | No | Only needed for client_credentials fallback |
| `email_password` | No | SMTP password (unused in Graph API mode) |

\* If no refresh_token, falls back to client_credentials (needs `Mail.Send` Application permission).

Secret URI format: `secrets://{env}/{tenant}/_/messaging-email/{key}`

### Azure AD App Requirements

The Azure AD app must be registered as a **public client** (no client_secret required for token exchange):
- **Delegated permissions**: `Mail.Send`, `offline_access`
- **Redirect URI**: `https://login.microsoftonline.com/common/oauth2/nativeclient` (Mobile/Desktop platform)
- **Public client flow**: Enabled in Authentication settings

### Acquiring a Refresh Token

Use the OAuth 2.0 authorization code flow with the nativeclient redirect URI:

```bash
# 1. Open this URL in browser and sign in:
TENANT="<tenant_id>"
CLIENT_ID="<client_id>"
echo "https://login.microsoftonline.com/${TENANT}/oauth2/v2.0/authorize?client_id=${CLIENT_ID}&response_type=code&redirect_uri=https%3A%2F%2Flogin.microsoftonline.com%2Fcommon%2Foauth2%2Fnativeclient&scope=https%3A%2F%2Fgraph.microsoft.com%2FMail.Send%20offline_access&response_mode=query"

# 2. After sign-in, copy the full URL from browser address bar (contains ?code=...)

# 3. Exchange code for tokens (no client_secret for public client):
curl -s -X POST "https://login.microsoftonline.com/${TENANT}/oauth2/v2.0/token" \
  --data-urlencode "grant_type=authorization_code" \
  --data-urlencode "code=<AUTH_CODE>" \
  --data-urlencode "client_id=${CLIENT_ID}" \
  --data-urlencode "redirect_uri=https://login.microsoftonline.com/common/oauth2/nativeclient" \
  --data-urlencode "scope=https://graph.microsoft.com/Mail.Send offline_access"
```

### Seeding Secrets

```bash
cat > /tmp/email-secrets.json << 'EOF'
{
  "entries": [
    {"uri": "secrets://dev/default/_/messaging-email/from_address", "format": "text", "value": {"type": "text", "text": "sender@yourdomain.com"}},
    {"uri": "secrets://dev/default/_/messaging-email/graph_tenant_id", "format": "text", "value": {"type": "text", "text": "<tenant-id>"}},
    {"uri": "secrets://dev/default/_/messaging-email/ms_graph_client_id", "format": "text", "value": {"type": "text", "text": "<client-id>"}},
    {"uri": "secrets://dev/default/_/messaging-email/ms_graph_refresh_token", "format": "text", "value": {"type": "text", "text": "<refresh-token>"}},
    {"uri": "secrets://dev/default/_/messaging-email/email_password", "format": "text", "value": {"type": "text", "text": "unused-graph-api-mode"}}
  ]
}
EOF

greentic-secrets apply --file /tmp/email-secrets.json \
  --store-path demo-bundle/.greentic/dev/.dev.secrets.env
```

## Destination Format

The `--to` argument accepts an email address:

```bash
GREENTIC_ENV=dev greentic-operator demo send \
  --bundle demo-bundle \
  --provider messaging-email \
  --to "recipient@example.com" \
  --text "Hello from Greentic!" \
  --tenant default --env dev
```

## Adaptive Card Support

**Tier**: TierD (downsampled to HTML)

Adaptive Cards are converted to an HTML email body. Text content is extracted as a fallback subject line when no explicit subject is provided.

## Egress Pipeline

```
render_plan  → TierD (extract text from AC if present)
encode       → extracts to/subject from envelope, serializes payload
send_payload → resolves secrets, exchanges refresh_token for access_token,
               POST /v1.0/me/sendMail via Graph API
```

## Ingress

The email provider handles inbound webhooks from Microsoft Graph notification subscriptions. Incoming emails are normalized to `HttpOutV1` events for the operator's egress pipeline.
