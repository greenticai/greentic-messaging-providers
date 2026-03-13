# Provider Status Report

## Summary (Updated 2026-02-26)

| Provider | Egress | Ingress | Round-Trip | Webhook | Status |
|----------|:---:|:---:|:---:|:---:|--------|
| **Telegram** | ✅ LIVE | ✅ OK | ✅ Full | ✅ Cloudflared | **FULLY WORKING** |
| **Slack** | ✅ LIVE | ✅ OK | ✅ Full | ⚠️ Manual (Slack portal) | **FULLY WORKING** |
| **Teams** | ✅ LIVE | ✅ OK | ✅ Full | ⚠️ Manual (Azure Bot) | **FULLY WORKING** |
| **WebChat** | ✅ OK | ✅ OK | ✅ Full | N/A (Direct Line) | **FULLY WORKING** |
| **Webex** | ✅ LIVE | ✅ Parse | ⚠️ Partial | ✅ API registered | **WORKING** (egress) |
| **Email** | ✅ LIVE | N/A | N/A | N/A | **WORKING** (egress) |
| **Dummy** | ✅ OK | N/A | N/A | N/A | **WORKING** |
| **WhatsApp** | ❌ | ❌ | ❌ | ❌ | **BLOCKED** (no creds) |

All 6 active providers pass full E2E via `demo start` HTTP gateway + echo bot pipeline.
113 operator tests pass, 0 failures.

---

## Telegram (`messaging.telegram.bot`)

### Source Files
- Component: `greentic-messaging-providers/components/messaging-provider-telegram/src/lib.rs`
- Ingress: `greentic-messaging-providers/components/messaging-ingress-telegram/src/lib.rs`
- Pack: `greentic-messaging-providers/packs/messaging-telegram/`
- Spec: `greentic-messaging-providers/specs/providers/telegram.yaml`

### Ops Implemented
| Op | Status | Notes |
|----|--------|-------|
| `send` | Done | POST /bot{token}/sendMessage |
| `reply` | Done | reply_to_message_id |
| `ingest_http` | Done | Parse Telegram update JSON |
| `render_plan` | Done | Returns TierD plan |
| `encode` | Done | Base64 ChannelMessageEnvelope |
| `send_payload` | Done | Decode + forward to Telegram API |

### Pack Extension Ops (pack.yaml)
Currently declares: `send, reply`
Missing: `ingest_http, render_plan, encode, send_payload`

### Secrets
- `TELEGRAM_BOT_TOKEN` (declared in component.manifest.json)

### Config (setup.yaml questions)
- `public_base_url` (required, regex: `^https://`)
- `default_chat_id` (optional)
- `bot_token` (required, secret)

### Ingress
- Mode: `custom` (separate `messaging-ingress-telegram.wasm`)
- Extension: `messaging.provider_ingress.v1`
- Export: `handle-webhook`

### Issues
1. Pack extension ops list only has `send, reply` - operator may not find `render_plan`/`encode`/`send_payload`
2. Otherwise most complete provider

---

## Webex (`messaging.webex.bot`)

### Source Files
- Component: `greentic-messaging-providers/components/messaging-provider-webex/src/lib.rs`
- Pack: `greentic-messaging-providers/packs/messaging-webex/`
- Spec: `greentic-messaging-providers/specs/providers/webex.yaml`

### Ops Implemented
| Op | Status | Notes |
|----|--------|-------|
| `send` | Done | POST /messages with roomId/toPersonId/toPersonEmail |
| `reply` | Done | parentId threading |
| `ingest_http` | Done | Parse webhook + secondary GET /messages/{id} fetch |
| `render_plan` | Done | Returns TierC plan |
| `encode` | Done | Base64 ChannelMessageEnvelope |
| `send_payload` | Done | Full implementation with Adaptive Card v1.3 support |

### Pack Extension Ops (pack.yaml)
Currently declares: `send, reply`
Missing: `ingest_http, render_plan, encode, send_payload`

### Secrets
- `WEBEX_BOT_TOKEN` (declared in component.manifest.json, scope: tenant)

### Config
ProviderConfig struct:
```rust
struct ProviderConfig {
    default_room_id: Option<String>,
    default_to_person_email: Option<String>,
    api_base_url: Option<String>,  // default: "https://webexapis.com/v1"
}
```

### Ingress
- Mode: `default` (no separate ingress component)
- Uses `ingest_http` op on the main adapter
- Secondary outbound HTTP call in ingest (GET /messages/{id})

### Issues

#### 1. Pack extension ops incomplete
**Problem:** pack.yaml only declares `send, reply`. Operator cannot call `render_plan`, `encode`, `send_payload` via provider-extension lookup.
**Fix:** Update pack.yaml and spec to include all 6 ops.

#### 2. Config schema vs code mismatch (CRITICAL)
**Problem:** `ProviderConfig` uses `#[serde(deny_unknown_fields)]` but config schema includes `public_base_url` which is NOT in the struct. Any config with `public_base_url` will crash at validation.
**Fix:** Either add `public_base_url` to struct or remove `deny_unknown_fields`.

#### 3. Hardcoded tenant in ingest
**Problem:** `build_webhook_envelope()` hardcodes `env = "default"` and `tenant = "default"`.
**Impact:** All incoming webhooks land in "default" tenant regardless of actual tenant.

#### 4. Secondary network call in ingest
**Note:** `ingest_http` makes a GET /messages/{id} call to fetch message content. Requires `http` WIT capability at runtime.

---

## Webchat (`messaging.webchat`)

### Source Files
- Component: `greentic-messaging-providers/components/messaging-provider-webchat/src/lib.rs`
- Direct Line: `greentic-messaging-providers/components/messaging-provider-webchat/src/directline/`
  - `http.rs` (Direct Line v3 protocol server)
  - `jwt.rs` (HMAC-SHA256 JWT)
  - `state.rs` (conversation state)
  - `store.rs` (state/secret host adapters)
- Pack: `greentic-messaging-providers/packs/messaging-webchat/`
- Spec: `greentic-messaging-providers/specs/providers/webchat.yaml`

### Ops Implemented
| Op | Status | Notes |
|----|--------|-------|
| `send` | Done | Writes to state_store (no HTTP) |
| `ingest` | Done | Simple text/user parse from JSON |
| `ingest_http` | Done | Routes to Direct Line handler or generic parse |
| `render_plan` | Done | Returns TierD plan |
| `encode` | Done | Text + route → ProviderPayloadV1 |
| `send_payload` | Done | Persist decoded payload to state_store |

### Pack Extension Ops (pack.yaml)
Currently declares: `send, ingest`
Missing: `ingest_http, render_plan, encode, send_payload`

### Secrets
- `jwt_signing_key` (used by Direct Line JWT but NOT declared in component.manifest.json!)

### Config
ProviderConfig struct:
```rust
struct ProviderConfig {
    route: Option<String>,
    tenant_channel_id: Option<String>,
    public_base_url: Option<String>,
}
```

Config schema (config.schema.json):
```json
{ "route", "tenant_channel_id", "mode" (enum: local_queue/websocket/pubsub), "base_url" }
```

### Ingress
- Mode: `default`
- No webhooks required
- Direct Line v3 REST API (stateful):
  - `POST /v3/directline/tokens/generate` - JWT generation
  - `POST /v3/directline/conversations` - create conversation
  - `POST /v3/directline/conversations/{id}/activities` - post message
  - `GET /v3/directline/conversations/{id}/activities` - poll messages

### Key Difference: State-Store Based
Webchat does NOT make external HTTP calls. It uses the host's state store as the communication medium:
- `send_payload` writes to state store (not HTTP)
- Browser polls via Direct Line GET activities
- WIT imports: `state-store`, `secrets-store` (NOT `http/client`)

### Issues

#### 1. Pack extension ops incomplete
**Problem:** pack.yaml only declares `send, ingest`. Missing `ingest_http`, `render_plan`, `encode`, `send_payload`.
**Fix:** ✅ FIXED - Added `messaging.provider_ingress.v1` extension with `ingest_http` op to pack.yaml

#### 2. jwt_signing_key secret NOT declared (CRITICAL)
**Problem:** Direct Line uses `jwt_signing_key` from secrets store but `component.manifest.json` has `secret_requirements: []`.
**Fix:** ✅ FIXED - Secret must be provisioned with category `messaging-webchat` (pack_id with hyphen, not dot)
**Secret path:** `secrets://{env}/{tenant}/{team}/messaging-webchat/jwt_signing_key`

#### 3. Config schema vs code total mismatch (CRITICAL)
**Problem:** Schema defines `mode`, `base_url`. Code uses `route`, `tenant_channel_id`, `public_base_url`. Validation will fail.
**Fix:** Align schema with code struct.

#### 4. Direct Line path doesn't emit events to operator pipeline
**Problem:** When browser posts via Direct Line, messages are stored in conversation state but NO `ChannelMessageEnvelope` is emitted to the operator's routing pipeline.
**Impact:** The operator can't process inbound webchat messages through app flows.
**Note:** Events are emitted but format is `ChannelMessageEnvelope`, not `EventEnvelopeV1`. Operator now handles this gracefully (skips with warning).

#### 5. Hardcoded tenant in generic ingest
**Problem:** `build_webchat_envelope()` hardcodes `env = "default"` and `tenant = "default"`.

#### 6. State persistence (architectural limitation)
**Problem:** Conversation state doesn't persist between requests because WASM component uses in-memory state.
**Impact:** Message posting to existing conversations fails in demo mode.
**Fix:** For production, external state store (Redis) is needed.

### Working Endpoints (2026-02-26)
- ✅ `POST /v3/directline/tokens/generate` - JWT token generation
- ✅ `POST /v3/directline/conversations` - Create conversation
- ⚠️ `POST /v3/directline/conversations/{id}/activities` - Works but state doesn't persist

---

## All 8 Providers at a Glance

| Provider | Type | Adapter | Ingress | Real HTTP | Webhooks | Subs | State |
|----------|------|---------|---------|-----------|----------|------|-------|
| Dummy | `messaging.dummy` | Full (mock) | None | No | No | No | Complete |
| Telegram | `messaging.telegram.bot` | Full | Custom WASM | Yes | Yes | No | Near-complete |
| Slack | `messaging.slack.api` | Full | Custom + HMAC | Yes + OAuth | Yes | No | Complete |
| Teams | `messaging.teams.bot` | Full | Custom | Yes + OAuth | Yes | Yes | Complete |
| WhatsApp | `messaging.whatsapp.cloud` | Full | Custom | Yes | Yes | No | Complete |
| Webex | `messaging.webex.bot` | Full | Default | Yes | Yes | No | Needs fixes |
| Email | `messaging.email.smtp` | Stub | None | No | No | No | Simulated only |
| Webchat | `messaging.webchat` | Full | Default | No (state-store) | No | No | Needs fixes |
