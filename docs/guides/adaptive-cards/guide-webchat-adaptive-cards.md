# Quick Start: Operator + WebChat for Adaptive Card Testing

End-to-end setup for testing adaptive card flows through the WebChat provider using `greentic-operator`. Covers cloning, building from source, configuring the demo bundle, and running the full pipeline.

## Prerequisites

| Tool | Version | Install |
|------|---------|---------|
| Rust toolchain | 1.91.0+ | `rustup default 1.91.0` |
| `wasm32-wasip2` target | - | `rustup target add wasm32-wasip2` |
| Node.js | 18+ | https://nodejs.org |
| `jq` | any | `apt install jq` / `brew install jq` |

## 1. Clone the Repos

Each repo is standalone. Clone the ones you need into a shared workspace directory:

```bash
mkdir -p greentic && cd greentic

git clone git@github.com:greentic-ai/greentic-operator.git
git clone git@github.com:greentic-ai/greentic-messaging-providers.git
git clone git@github.com:greentic-ai/greentic-webchat.git
```

Optional (only if you need to rebuild the runner host library):

```bash
git clone git@github.com:greentic-ai/greentic-runner.git
```

After cloning, your workspace should look like:

```
greentic/
├── greentic-operator/
├── greentic-messaging-providers/
├── greentic-webchat/
└── greentic-runner/          # optional
```

## 2. Build the Operator from Source

```bash
cd greentic-operator
cargo build --release
```

The binary is at `target/release/greentic-operator`. Optionally install it:

```bash
cargo install --path .
# Now available as: greentic-operator
```

### Build the seed-secret tool

```bash
cargo build --manifest-path tools/seed-secret/Cargo.toml --release
```

Binary at `tools/seed-secret/target/release/seed-secret`.

## 3. Build Provider WASM Components

```bash
cd greentic-messaging-providers

# Ensure wasm32-wasip2 target is installed
rustup target add wasm32-wasip2

# Build all 24 WASM components
SKIP_WASM_TOOLS_VALIDATION=1 ./tools/build_components.sh
```

Built WASMs land in `target/components/`. To build only the webchat provider:

```bash
cargo build --release --package messaging-provider-webchat --target wasm32-wasip2
```

### Build the webchat gtpack

```bash
# If packc (greentic-pack) is installed:
cd packs/messaging-webchat
greentic-pack build --in . --allow-pack-schema --offline

# Output: dist/messaging-webchat.gtpack
```

If `greentic-pack` is not installed, use a pre-built gtpack from `packs/messaging-webchat/dist/` or copy from `demo-bundle/providers/messaging/`.

> **Stale WASM cache warning:** If you change dependencies and rebuild, always clean the target first:
> ```bash
> rm -rf target/wasm32-wasip2/
> ```
> This avoids stale artifacts from the Cargo incremental cache.

## 4. Prepare the Demo Bundle

The demo bundle is the working directory for the operator:

```
demo-bundle/
├── packs/
│   └── default.gtpack              # App pack (Cisco bundle or your flow)
├── providers/
│   └── messaging/
│       └── messaging-webchat.gtpack # WebChat provider pack
├── tenants/
│   └── default/
│       ├── tenant.gmap
│       └── teams/
│           └── default/
│               └── team.gmap
└── .greentic/
    └── dev/
        └── .dev.secrets.env         # Created by seed-secret
```

### Create the directory structure

```bash
cd /path/to/greentic   # workspace root

mkdir -p demo-bundle/packs
mkdir -p demo-bundle/providers/messaging
mkdir -p demo-bundle/tenants/default/teams/default
mkdir -p demo-bundle/.greentic/dev
```

### Create tenant access maps

```bash
# Allow webchat provider for this tenant
cat > demo-bundle/tenants/default/tenant.gmap << 'EOF'
messaging-webchat = public
EOF

cat > demo-bundle/tenants/default/teams/default/team.gmap << 'EOF'
messaging-webchat = public
EOF
```

### Copy the provider pack

```bash
# WebChat provider pack (from the messaging-providers build output)
cp greentic-messaging-providers/packs/messaging-webchat/dist/messaging-webchat.gtpack \
   demo-bundle/providers/messaging/
```

### Build and copy the app pack (Cisco bundle)

The operator needs an app pack (`default.gtpack`) to process messages and route Adaptive Card responses. The Cisco bundle contains card-based flows generated from Adaptive Card JSON files.

#### Source files

The Cisco demo provides two ZIP files:

| File | Contents |
|------|----------|
| `cisco-live.zip` | 44 Adaptive Card JSON files (the raw cards) |
| `cisco-bundle.zip` | Generated pack workspace (cards + flows + built gtpack) |

`cisco-live.zip` contains the card JSONs directly:

```
cisco-live/
├── RFQ-CARD-01_intake.json
├── RFQ-CARD-02_processing.json
├── MP-CARD-01_id_entry.json
├── HRM-CARD-01_menu.json
├── NT-CARD-01_incident.json
└── ... (44 cards total)
```

`cisco-bundle.zip` contains the full generated pack workspace:

```
cisco-bundle/
├── pack.yaml
├── flows/
│   └── main.ygtc              # Generated flow (card routing graph)
├── assets/
│   └── cards/                  # All 44 card JSONs
├── components/                 # WASM components (added during build)
└── dist/
    └── cisco-bundle.gtpack     # Built pack (ready to deploy)
```

#### Card JSON format

Cards are standard Adaptive Card v1.3 JSON. Routing between cards is done via `Action.Submit` data fields:

```json
{
  "$schema": "http://adaptivecards.io/schemas/adaptive-card.json",
  "type": "AdaptiveCard",
  "version": "1.3",
  "body": [
    {"type": "TextBlock", "text": "RFQ intake", "size": "ExtraLarge", "weight": "Bolder"}
  ],
  "actions": [
    {
      "type": "Action.Submit",
      "title": "Start compliance scan",
      "data": {
        "flow": "rfq",
        "cardId": "RFQ-CARD-01",
        "step": "startScan",
        "routeToCardId": "RFQ-CARD-02_processing"
      }
    }
  ]
}
```

Key fields in `actions[].data`:
- `flow`: which flow this card belongs to (e.g. `rfq`, `meetingPrep`)
- `cardId`: this card's ID
- `step`: the action/step name
- `routeToCardId`: next card to render after this action

`greentic-cards2pack` reads these fields to generate the flow graph (`.ygtc`).

#### Option A: Use pre-built gtpack

If you have `cisco-bundle.zip` with the built gtpack:

```bash
unzip cisco-bundle.zip
cp cisco-bundle/dist/cisco-bundle.gtpack demo-bundle/packs/default.gtpack
```

#### Option B: Generate from card JSONs

Build from `cisco-live.zip` (raw cards only):

```bash
# 1. Build greentic-cards2pack
cd greentic-cards2pack
cargo build --release

# 2. Extract cards
unzip cisco-live.zip -d /tmp/

# 3. Generate pack workspace + build gtpack
target/release/greentic-cards2pack generate \
  --cards /tmp/cisco-live \
  --out /tmp/cisco-bundle \
  --name cisco-bundle \
  --verbose

# 4. Deploy
cp /tmp/cisco-bundle/dist/cisco-bundle.gtpack demo-bundle/packs/default.gtpack
```

#### Updating cards in an existing gtpack

If you only changed card JSON files (no flow changes):

```bash
tmpdir=$(mktemp -d)
for card in assets/cards/*.json; do
  mkdir -p "$tmpdir/$(dirname "$card")"
  cp "$card" "$tmpdir/$card"
done
(cd "$tmpdir" && zip -u /path/to/cisco-bundle.gtpack assets/cards/*.json)
rm -rf "$tmpdir"

cp cisco-bundle.gtpack demo-bundle/packs/default.gtpack
```

## 5. Seed the JWT Signing Key

The WebChat provider uses a `jwt_signing_key` secret to sign Direct Line JWTs. Without this, `/token` returns `secret error: not-found`.

The secret URI format is: `secrets://{env}/{tenant}/{team}/{category}/{key}`

- `env`: `dev` (matches `GREENTIC_ENV`)
- `tenant`: `default` (matches your tenant directory)
- `team`: `_` (wildcard, applies to all teams)
- `category`: `messaging-webchat` (matches the provider pack ID)
- `key`: the secret name

```bash
# Seed the JWT signing key
tools/seed-secret/target/release/seed-secret \
  demo-bundle/.greentic/dev/.dev.secrets.env \
  "secrets://dev/default/_/messaging-webchat/jwt_signing_key" "my-dev-signing-key-2026"
```

> **DEK cache bug:** The secrets backend caches encryption keys by `(env, tenant, team, category)`. If you need to seed multiple secrets for the same provider, pass them all in **one invocation**:
> ```bash
> seed-secret demo-bundle/.greentic/dev/.dev.secrets.env \
>   "secrets://dev/default/_/messaging-webchat/jwt_signing_key" "my-key" \
>   "secrets://dev/default/_/messaging-webchat/another_secret" "another-value"
> ```
> Separate invocations for the same category generate different encryption keys, making earlier secrets unreadable. The QA onboard UI handles this automatically.

## 6. Start the Operator

```bash
GREENTIC_ENV=dev gtc op demo start --bundle demo-bundle
```

`GREENTIC_ENV=dev` tells the operator to use the `dev` environment. This determines:
- Which secrets file to read: `.greentic/dev/.dev.secrets.env`
- The `env` segment in secret URIs: `secrets://dev/...`

Expected output:

```
HTTP ingress ready at http://127.0.0.1:8080
demo start running (bundle=demo-bundle targets=[default]); press Ctrl+C to stop
```

The operator exposes:

| Endpoint | Purpose |
|----------|---------|
| `POST /token` | Direct Line JWT issuance |
| `POST /v3/directline/conversations` | Create conversation |
| `POST /v3/directline/conversations/{id}/activities` | Send activity |
| `GET /v3/directline/conversations/{id}/activities` | Poll activities |
| `GET /api/onboard/providers` | List available providers |
| `POST /api/onboard/qa/spec` | Get setup form spec |
| `POST /api/onboard/qa/submit` | Submit setup answers |

## 7. Configure via Onboard QA API (Optional)

Instead of manual secret seeding, submit all config via the onboard API (operator must be running):

```bash
curl -s -X POST http://localhost:8080/api/onboard/qa/submit \
  -H "Content-Type: application/json" \
  -d '{
    "provider_id": "messaging-webchat",
    "domain": "messaging",
    "tenant": "default",
    "answers": {
      "public_base_url": "https://your-tunnel.trycloudflare.com",
      "mode": "directline",
      "jwt_signing_key": "my-dev-signing-key-2026"
    }
  }' | jq .
```

This persists all secrets and runs the provider's `setup_default` flow in one call.

## 8. Start the WebChat SPA

```bash
# Terminal 3
cd greentic-webchat/apps/webchat-spa
npm install
npm run dev    # http://localhost:5174
```

The Vite config (`vite.config.ts`) proxies these paths to the operator at :8080:

- `/token` -> `http://localhost:8080/token`
- `/v3/directline/*` -> `http://localhost:8080/v3/directline/*`

### Open in browser

```
http://localhost:5174/dev
```

The `/dev` path loads the `dev` skin which uses the Vite proxy for Direct Line.

> **Cisco skin:** `http://localhost:5174/cisco` loads the full-page Cisco-branded layout.
> To use the Cisco skin locally, edit `public/skins/cisco/skin.json`:
> ```json
> "directLine": {
>   "tokenUrl": "http://127.0.0.1:8080/token?tenant=default",
>   "domain": "http://127.0.0.1:8080/v3/directline"
> }
> ```

## 9. Test Adaptive Cards

### Option A: Type in the SPA

Type a message in the WebChat box. The Cisco bundle flow processes the message and routes it through card nodes that render Adaptive Card responses back to WebChat.

### Option B: Send a card from CLI

```bash
# Create a test card
cat > /tmp/test-card.json << 'CARD'
{
  "type": "AdaptiveCard",
  "version": "1.3",
  "body": [
    {"type": "TextBlock", "text": "Hello from Greentic!", "weight": "bolder", "size": "large"},
    {"type": "TextBlock", "text": "This is an adaptive card rendered in WebChat."},
    {"type": "FactSet", "facts": [
      {"title": "Provider", "value": "WebChat"},
      {"title": "Protocol", "value": "Direct Line v3"},
      {"title": "Card Version", "value": "1.3"}
    ]}
  ],
  "actions": [
    {"type": "Action.Submit", "title": "Click Me", "data": {"action": "test"}}
  ]
}
CARD

# Get a conversation ID from browser DevTools (Network tab, look for /conversations request)
GREENTIC_ENV=dev gtc op demo send \
  --bundle demo-bundle \
  --provider messaging-webchat \
  --to "<conversation-id>" \
  --text "Card fallback text" \
  --card /tmp/test-card.json \
  --tenant default
```

### Option C: Verify with curl

```bash
# 1. Get token
TOKEN=$(curl -s -X POST http://localhost:8080/token | jq -r .token)

# 2. Create conversation
CONV=$(curl -s -X POST http://localhost:8080/v3/directline/conversations \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json")
CONV_ID=$(echo "$CONV" | jq -r .conversationId)
CONV_TOKEN=$(echo "$CONV" | jq -r .token)
echo "Conversation: $CONV_ID"

# 3. Send a message
curl -s -X POST "http://localhost:8080/v3/directline/conversations/$CONV_ID/activities" \
  -H "Authorization: Bearer $CONV_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"type":"message","text":"hello","from":{"id":"user1"}}' | jq .

# 4. Poll for bot response (wait 2s for app flow)
sleep 2
curl -s "http://localhost:8080/v3/directline/conversations/$CONV_ID/activities" \
  -H "Authorization: Bearer $CONV_TOKEN" | jq '.activities[] | {from: .from.id, text, type}'
```

## Architecture

```
Browser (WebChat SPA :5174)
    |
    |  Direct Line v3 protocol
    |  POST /token -> JWT
    |  POST /v3/directline/conversations/{id}/activities
    |  GET  /v3/directline/conversations/{id}/activities (poll)
    v
Operator HTTP Gateway (:8080)
    |
    +-- /api/onboard/*     -> Provider setup wizard
    +-- /token             -> Direct Line JWT issuance
    +-- /v3/directline/*   -> Direct Line API
    +-- /v1/messaging/ingress/* -> Webhook ingress
    |
    v
WASM Provider Components (in .gtpack)
    |  ingest_http -> ChannelMessageEnvelope
    v
App Flow (cisco-bundle)
    |  render_plan -> encode -> send_payload
    v
WebChat Direct Line response (Adaptive Card attachment)
```

The pipeline for a user message:

1. SPA sends activity via Direct Line API
2. Operator routes to webchat provider's `ingest_http` op
3. Provider returns `ChannelMessageEnvelope` events
4. Operator routes envelopes through the app flow (`default.gtpack`)
5. App flow returns response envelope
6. Operator calls provider's `render_plan` -> `encode` -> `send_payload`
7. Provider writes response activity to Direct Line state store
8. SPA polls and receives the response

## Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| `secret error: not-found` on `/token` | `jwt_signing_key` not seeded | Run seed-secret or configure via onboard UI |
| `WebChat mount node not found` | DOMPurify strips `id` attribute | Ensure `sanitizeShellHtml.ts` ALLOWED_ATTR includes `id` |
| SPA shows "Connecting..." forever | Token endpoint unreachable | Check Vite proxy config, ensure operator runs on :8080 |
| `401 Unauthorized` on activities | Using initial token instead of conversation token | Use the `token` from `/conversations` response, not `/token` |
| Card not rendering in WebChat | `Action.Execute` unsupported | SPA middleware converts `Action.Execute` -> `Action.Submit` automatically |
| No bot response after sending | App pack missing or flow error | Check operator stdout for `app flow failed` errors |
| CORS error in browser | Direct request to :8080 | Use Vite dev server (not direct :8080) -- it proxies with correct headers |
| Stale WASM behavior | Cached old wasm32-wasip2 artifacts | `rm -rf target/wasm32-wasip2/` and rebuild |

## Port Summary

| Service | Port | Purpose |
|---------|------|---------|
| Operator | 8080 | HTTP gateway (Direct Line + onboard API + ingress) |
| Onboard QA UI | 5173 | Provider setup wizard |
| WebChat SPA | 5174 | WebChat frontend (or 5173 if onboard not running) |

## Quick Checklist

- [ ] Repos cloned (`greentic-operator`, `greentic-messaging-providers`, `greentic-webchat`)
- [ ] Operator built (`cargo build --release` in greentic-operator)
- [ ] `demo-bundle/providers/messaging/messaging-webchat.gtpack` exists
- [ ] `demo-bundle/packs/default.gtpack` exists (app pack)
- [ ] `demo-bundle/tenants/default/tenant.gmap` has `messaging-webchat = public`
- [ ] `jwt_signing_key` seeded (via seed-secret or onboard UI)
- [ ] `GREENTIC_ENV=dev` is set
- [ ] Operator running on :8080
- [ ] WebChat SPA running on :5174 (or :5173)
- [ ] Browser open at `http://localhost:5174/dev`
- [ ] Type a message -> bot responds -> card renders
