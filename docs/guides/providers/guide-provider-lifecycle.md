# Provider Lifecycle: Add, Setup, Update, Remove

How messaging providers are discovered, configured, updated, and removed in `greentic-operator`.

## Overview

A **provider** is a `.gtpack` archive containing WASM components and flows that bridge external messaging services (Telegram, Slack, Teams, WebChat, etc.) into the Greentic platform. The operator discovers providers from the bundle's `providers/messaging/` directory.

## Provider Pack Structure

Each `.gtpack` is a ZIP archive:

```
messaging-telegram.gtpack (Capability-Driven Pattern)
├── pack.yaml                # Pack manifest (components, flows, extensions)
├── components/
│   ├── messaging-provider-telegram/
│   │   └── component.wasm   # Core provider (send/reply/qa-spec/apply-answers/i18n-keys)
│   └── messaging-ingress-telegram/
│       └── component.wasm   # Webhook handler
├── flows/
│   ├── setup_default.ygtc                 # Single-node: invoke messaging.configure
│   ├── setup_default.ygtc.resolve.json
│   ├── requirements.ygtc                  # Single-node: invoke messaging.configure
│   └── requirements.ygtc.resolve.json
├── assets/
│   ├── setup.yaml                         # QA wizard form definition
│   └── schemas/messaging/telegram/
│       └── public.config.schema.json
└── fixtures/                              # Test fixtures
```

## 1. Add a Provider

### Step 1: Get the gtpack

Provider gtpacks are built from the `greentic-messaging-providers` repo:

```bash
cd greentic-messaging-providers

# Build all WASMs
SKIP_WASM_TOOLS_VALIDATION=1 ./tools/build_components.sh

# Build all packs (outputs to packs/*/dist/*.gtpack)
./tools/build_packs.sh

# Or build a single pack
cd packs/messaging-telegram
greentic-pack build --in . --allow-pack-schema --offline
# Output: dist/messaging-telegram.gtpack
```

If you don't have `greentic-pack` installed, use pre-built gtpacks from `packs/messaging-telegram/dist/`.

### Step 2: Place the gtpack in the bundle

```bash
cp greentic-messaging-providers/packs/messaging-telegram/dist/messaging-telegram.gtpack \
   demo-bundle/providers/messaging/
```

The operator scans `providers/messaging/` for `*.gtpack` files on startup.

### Step 3: Create tenant access maps

The tenant must grant access to the provider:

```bash
# tenant.gmap: which providers this tenant can use
echo "messaging-telegram = public" >> demo-bundle/tenants/default/tenant.gmap

# team.gmap: which providers this team can use
echo "messaging-telegram = public" >> demo-bundle/tenants/default/teams/default/team.gmap
```

### Step 4: Verify discovery

```bash
# Via CLI
GREENTIC_ENV=dev gtc op demo start --bundle demo-bundle
# Look for: "loaded provider pack messaging-telegram" in stdout

# Via API (while operator is running)
curl -s http://localhost:8080/api/onboard/providers | jq '.providers[].pack_id'
```

API response:

```json
{
  "providers": [
    {
      "pack_id": "messaging-telegram",
      "domain": "messaging",
      "file_name": "messaging-telegram.gtpack",
      "display_name": "Telegram",
      "entry_flows": ["setup_default", "verify_webhooks"]
    }
  ]
}
```

## 2. Setup (Configure) a Provider

Setup seeds secrets and runs provider-specific initialization flows (e.g., register webhooks).

### Via API

The QA flow has 3 stages: **spec** -> **validate** -> **submit**.

```bash
# 1. Get the setup form spec
curl -s -X POST http://localhost:8080/api/onboard/qa/spec \
  -H "Content-Type: application/json" \
  -d '{
    "provider_id": "messaging-telegram",
    "domain": "messaging",
    "tenant": "default",
    "answers": {},
    "locale": "en"
  }' | jq .
```

Response includes questions with titles, types, validation rules:

```json
{
  "questions": [
    {
      "name": "public_base_url",
      "title": "Public base URL",
      "kind": "string",
      "required": true,
      "secret": false,
      "help": "Example: https://xxxx.trycloudflare.com"
    },
    {
      "name": "bot_token",
      "title": "Telegram bot token",
      "kind": "string",
      "required": true,
      "secret": true
    }
  ]
}
```

```bash
# 2. Validate answers (optional, checks constraints before persisting)
curl -s -X POST http://localhost:8080/api/onboard/qa/validate \
  -H "Content-Type: application/json" \
  -d '{
    "provider_id": "messaging-telegram",
    "domain": "messaging",
    "tenant": "default",
    "answers": {
      "public_base_url": "https://my-tunnel.trycloudflare.com",
      "bot_token": "1234567890:AAF..."
    }
  }' | jq .

# 3. Submit (persists secrets + runs setup_default + verify_webhooks flows)
curl -s -X POST http://localhost:8080/api/onboard/qa/submit \
  -H "Content-Type: application/json" \
  -d '{
    "provider_id": "messaging-telegram",
    "domain": "messaging",
    "tenant": "default",
    "answers": {
      "public_base_url": "https://my-tunnel.trycloudflare.com",
      "bot_token": "1234567890:AAF..."
    }
  }' | jq .
```

### Option C: Via seed-secret (manual)

For providers where you already have credentials:

```bash
# Build seed-secret (first time, from greentic-operator repo)
cargo build --manifest-path tools/seed-secret/Cargo.toml --release
```

**Secret URI format:** `secrets://{env}/{tenant}/{team}/{category}/{key}`

| Segment | Example | Notes |
|---------|---------|-------|
| `env` | `dev` | Matches `GREENTIC_ENV` |
| `tenant` | `default` | Your tenant directory name |
| `team` | `_` | `_` = wildcard (all teams), or specific team name |
| `category` | `messaging-telegram` | Usually matches the provider pack ID |
| `key` | `telegram_bot_token` | The secret name the WASM component reads |

**All secrets for the same category MUST be seeded in one invocation** (pass multiple URI-value pairs):

```bash
tools/seed-secret/target/release/seed-secret \
  demo-bundle/.greentic/dev/.dev.secrets.env \
  "secrets://dev/default/_/messaging-telegram/telegram_bot_token" "1234567890:AAF..." \
  "secrets://dev/default/_/messaging-telegram/public_base_url" "https://my-tunnel.trycloudflare.com"
```

> **Why one invocation?** The secrets backend caches encryption keys (DEKs) by `(env, tenant, team, category)` without the secret name. Separate process invocations generate different DEKs, making secrets from earlier invocations unreadable. The QA onboard UI and `seed-secret` batch mode handle this correctly.

### What happens during submit

1. **Persist secrets**: All form answers are written to the secrets store (`demo-bundle/.greentic/dev/.dev.secrets.env`)
2. **Secret aliasing**: Some providers need field name mapping (e.g., `bot_token` -> `telegram_bot_token`). The wizard handles this automatically.
3. **Run `setup_default` flow**: Provider-specific setup (e.g., register webhooks, validate API keys)
4. **Run `verify_webhooks` flow**: Verifies external service configuration is correct
5. **Write config**: Provider config is persisted for runtime use

### Form spec loading priority

The wizard loads the setup form from two sources with this priority:

1. **`assets/setup.yaml`** inside the gtpack (preferred, easy to update without WASM rebuild)
2. **WASM `qa-spec` op** from the provider component (fallback)

### setup.yaml format reference

The `assets/setup.yaml` file inside a gtpack defines the setup form:

```yaml
provider_id: telegram       # Provider identifier (without messaging- prefix)
version: 1                  # Schema version
title: Telegram provider setup
questions:
  - name: bot_token          # Field name (used as secret key)
    title: Bot token          # Display label
    kind: string              # Field type: string, boolean, number
    required: true            # Whether the field is mandatory
    secret: true              # true = stored encrypted, masked in UI
    help: "Get from @BotFather"   # Help text shown below the field
    validate:                 # Optional validation
      regex: "^[0-9]+:.*"    # Regex the value must match
  - name: public_base_url
    title: Public base URL
    kind: string
    required: true
    secret: false
    help: "Example: https://xxxx.trycloudflare.com"
    validate:
      regex: "^https://"
```

Fields with `secret: true` are stored encrypted. Fields with `secret: false` are also stored in the secrets backend (because WASM components read all config via the secrets API).

## 3. Update a Provider

### Update secrets

Re-run the submit flow with new values. The wizard overwrites existing secrets:

```bash
curl -s -X POST http://localhost:8080/api/onboard/qa/submit \
  -H "Content-Type: application/json" \
  -d '{
    "provider_id": "messaging-telegram",
    "domain": "messaging",
    "tenant": "default",
    "answers": {
      "public_base_url": "https://new-tunnel.trycloudflare.com",
      "bot_token": "NEW_TOKEN_HERE"
    }
  }' | jq .
```

This re-runs `setup_default` and `verify_webhooks`, which for Telegram re-registers the webhook URL with Telegram's Bot API.

### Update the gtpack

To deploy a new version of the provider pack:

```bash
# Replace the gtpack
cp new-messaging-telegram.gtpack demo-bundle/providers/messaging/messaging-telegram.gtpack

# Restart the operator to pick up the new pack
# (Ctrl+C the running operator, then start again)
GREENTIC_ENV=dev gtc op demo start --bundle demo-bundle
```

### Update a WASM inside an existing gtpack

If you only rebuilt one component:

```bash
# Create temp dir with correct zip structure
tmpdir=$(mktemp -d)
mkdir -p "$tmpdir/components/messaging-provider-telegram"
cp target/components/messaging-provider-telegram/component.wasm "$tmpdir/components/messaging-provider-telegram/component.wasm"

# Update the zip in-place
(cd "$tmpdir" && zip -u /path/to/messaging-telegram.gtpack components/messaging-provider-telegram/component.wasm)
rm -rf "$tmpdir"

# Verify
zipinfo messaging-telegram.gtpack | grep wasm
```

## 4. Remove a Provider

### Remove from bundle

```bash
rm demo-bundle/providers/messaging/messaging-telegram.gtpack
```

### Remove tenant access

Edit the `.gmap` files to remove the provider line:

```bash
# Remove "messaging-telegram = public" from both files
sed -i '/messaging-telegram/d' demo-bundle/tenants/default/tenant.gmap
sed -i '/messaging-telegram/d' demo-bundle/tenants/default/teams/default/team.gmap
```

### Clean up secrets (optional)

Secrets are stored in `demo-bundle/.greentic/dev/.dev.secrets.env`. The secrets file is encrypted, so you can either:

- Delete the entire secrets file and re-seed all providers: `rm demo-bundle/.greentic/dev/.dev.secrets.env`
- Leave it (orphaned secrets are harmless, they just take up space)

### Restart operator

```bash
GREENTIC_ENV=dev gtc op demo start --bundle demo-bundle
```

## Provider-Specific Setup Notes

### Telegram
- Get bot token from @BotFather on Telegram
- Needs public URL for webhook (use `cloudflared tunnel` or ngrok)
- Webhook registered automatically by `setup_default` flow

### Slack
- Create Slack app at https://api.slack.com/apps
- Needs `slack_bot_token` (xoxb-...) and `slack_app_id`
- Events API URL must be set manually in Slack developer portal: `https://<public-url>/v1/messaging/ingress/messaging-slack/<tenant>/webhook`

### Teams
- Register Azure AD app (public client, no client secret)
- Needs `tenant_id`, `client_id`, `refresh_token`
- Get refresh token via OAuth authorization_code flow with delegated permissions

### WebChat
- Needs `jwt_signing_key` for Direct Line token signing
- No external API keys needed
- Uses state-store WIT interface for conversation persistence

### Email (Microsoft Graph)
- Needs `from_address`, `graph_tenant_id`, `ms_graph_client_id`
- Optional: `ms_graph_refresh_token` (delegated) or `ms_graph_client_secret` (app)
- No webhook ingress (send-only)

### Webex
- Get bot token from https://developer.webex.com
- Needs `webex_bot_token`
- Webhook registered by `setup_default` flow

## Checking Deployment Status

```bash
curl -s http://localhost:8080/api/onboard/status | jq .
```

```json
{
  "deployed": [
    {
      "provider_id": "messaging-telegram",
      "configured": true,
      "config_files": ["default.json"],
      "instance_label": "Telegram Bot",
      "scope_tenant": "default"
    }
  ]
}
```

## Provider Flow Summary

| Flow | When | Purpose |
|------|------|---------|
| `setup_default` | On submit | Register webhooks, validate API keys |
| `verify_webhooks` | After setup | Verify external service config is correct |
| `setup_qa` | On spec request | Generate setup form (WASM-based) |
| `requirements` | On send --print-required-args | Show required config values |
| `update` | On re-submit | Update existing config |
| `diagnostics` | Manual | Run diagnostic checks |
| `sync_subscriptions` | Manual | Sync event subscriptions |
| `default` | On message | Handle incoming/outgoing messages |
