# Operator ↔ Messaging Provider Integration Guide

## Overview

This document explains how `greentic-operator` integrates with messaging provider WASM components from `greentic-messaging-providers`. The operator acts as the host runtime that loads, instantiates, and invokes provider components for sending/receiving messages.

```
┌─────────────────────────────────────────────────────────────────┐
│                      greentic-operator                           │
│                                                                  │
│  CLI (demo send/ingress/start)                                   │
│       │                                                          │
│       ▼                                                          │
│  DemoRunnerHost                                                  │
│       │                                                          │
│       ├── discover_provider_packs()  ← reads .gtpack from bundle │
│       ├── PackRuntime::load()        ← loads WASM via Wasmtime   │
│       └── invoke_provider()          ← calls WASM operations     │
│              │                                                   │
│              ▼                                                   │
│  ┌──────────────────────────────────────────┐                    │
│  │  Wasmtime Component Model                 │                    │
│  │                                           │                    │
│  │  Host provides:                           │                    │
│  │    ├── greentic:http/http-client@1.1.0    │   ← outgoing HTTP │
│  │    ├── greentic:secrets/store@1.0.0       │   ← secret lookup │
│  │    └── greentic:state/state-store@1.0.0   │   ← key-value     │
│  │                                           │                    │
│  │  Guest exports:                           │                    │
│  │    ├── greentic:component/runtime@0.6.1   │   ← CBOR invoke   │
│  │    └── greentic:provider/schema-core-api  │   ← JSON invoke   │
│  └──────────────────────────────────────────┘                    │
└─────────────────────────────────────────────────────────────────┘
```

---

## 1. Pack Loading & Component Instantiation

### What is a .gtpack?

A `.gtpack` file is a ZIP archive containing:

```
messaging-telegram.gtpack (ZIP)
├── manifest.cbor                              ← Provider metadata (binary CBOR)
├── components/
│   ├── messaging-provider-telegram.wasm       ← WASM component binary
│   └── templates.wasm                         ← Shared templates component
├── flows/
│   ├── setup_default.ygtc                     ← Provider setup flow
│   ├── render_plan.ygtc                       ← Rendering flow
│   ├── encode.ygtc                            ← Encoding flow
│   └── send_payload.ygtc                      ← Sending flow
├── assets/
│   ├── setup.yaml                             ← QA setup questions
│   └── i18n/en.ftl                            ← Localization strings
└── secret-requirements.json                   ← Required secrets list
```

### How the Operator Loads Packs

**File**: `greentic-operator/src/demo/runner_host.rs`

```
DemoRunnerHost::new(bundle_path)
    │
    ├── 1. Scan bundle/providers/messaging/*.gtpack
    ├── 2. Read manifest.cbor from each ZIP
    ├── 3. Build catalog: HashMap<(Domain, ProviderType), ProviderPack>
    │      Key example: (Messaging, "messaging.telegram.bot")
    │
    └── 4. On invoke: PackRuntime::load()
              │
              ├── Extract WASM bytes from ZIP
              ├── Compile via Wasmtime (component model)
              ├── Create Linker with host imports:
              │     ├── HTTP client (outgoing requests)
              │     ├── Secrets store (read-only)
              │     ├── State store (read/write) ← optional
              │     └── WASI preview 2 (clock, random, etc.)
              ├── Instantiate component
              └── Return PackRuntime handle
```

### Host Capabilities Injected into WASM

| Capability | WIT Interface | What It Does |
|-----------|---------------|-------------|
| HTTP Client | `greentic:http/http-client@1.1.0` | Provider makes outgoing HTTP calls (Telegram API, Slack API, etc.) |
| Secrets Store | `greentic:secrets/store@1.0.0` | Provider reads bot tokens, API keys from encrypted store |
| State Store | `greentic:state/state-store@1.0.0` | Optional host KV store for components that declare the import |
| WASI | `wasi:http`, `wasi:clocks`, `wasi:random` | Standard WASI capabilities |

---

## 2. Outbound Message Flow (demo send)

### CLI Command

```bash
GREENTIC_ENV=dev greentic-operator demo send \
  --bundle /path/to/demo-bundle \
  --provider messaging-telegram \
  --to "7951102355" \
  --text "Hello from Greentic" \
  --card /tmp/card.json \
  --tenant default
```

### Full Pipeline

```
CLI args
  │
  ▼
┌─────────────────────────────────────────────────────────┐
│ 1. RESOLVE PACK                                          │
│    discover_provider_packs_cbor_only(bundle_path)        │
│    Filter by tenant/team via gmap policy                 │
│    Find .gtpack for "messaging-telegram"                 │
└─────────────────────┬───────────────────────────────────┘
                      │
                      ▼
┌─────────────────────────────────────────────────────────┐
│ 2. BUILD MESSAGE                                         │
│    RenderPlanInV1 {                                      │
│      v: 1,                                               │
│      message: {                                          │
│        text: "Hello from Greentic",                      │
│        to: [{ id: "7951102355" }],                       │
│        metadata: {                                       │
│          adaptive_card: "{...}"    ← if --card provided  │
│        }                                                 │
│      }                                                   │
│    }                                                     │
└─────────────────────┬───────────────────────────────────┘
                      │
                      ▼
┌─────────────────────────────────────────────────────────┐
│ 3. RENDER PLAN (WASM invoke)                             │
│    op: "render_plan"                                     │
│                                                          │
│    Provider does:                                        │
│    ├── Parse RenderPlanInV1                               │
│    ├── Check for adaptive_card in metadata               │
│    ├── If AC: extract_planner_card() → plan_render()     │
│    │   ├── Determine tier (A/B/D based on capabilities)  │
│    │   ├── Extract text summary from AC elements         │
│    │   └── Truncate to max_text_len (4096 for Telegram)  │
│    └── Return RenderPlanOutV1 { plan_json }              │
│                                                          │
│    Output: { tier: "D", summary_text: "...",             │
│              warnings: ["adaptive_card_downsampled"] }    │
└─────────────────────┬───────────────────────────────────┘
                      │
                      ▼
┌─────────────────────────────────────────────────────────┐
│ 4. ENCODE (WASM invoke)                                  │
│    op: "encode"                                          │
│                                                          │
│    Provider does:                                        │
│    ├── Parse EncodeInV1 { message, plan }                │
│    ├── Build ChannelMessageEnvelope:                     │
│    │   ├── channel: "telegram"                           │
│    │   ├── to: "7951102355"                              │
│    │   ├── text: "Hello from Greentic"                   │
│    │   └── metadata: { adaptive_card: "..." }            │
│    ├── For non-AC providers (Telegram):                  │
│    │   └── extract_ac_summary() → replace text with      │
│    │       downsampled AC content                        │
│    └── Return JSON-serialized envelope bytes             │
│                                                          │
│    Output: ProviderPayloadV1 { payload: <bytes> }        │
└─────────────────────┬───────────────────────────────────┘
                      │
                      ▼
┌─────────────────────────────────────────────────────────┐
│ 5. SEND PAYLOAD (WASM invoke)                            │
│    op: "send_payload"                                    │
│                                                          │
│    Provider does:                                        │
│    ├── Deserialize ChannelMessageEnvelope                │
│    ├── Read bot_token from secrets store                 │
│    │   secrets://dev/default/_/messaging-telegram/       │
│    │            telegram_bot_token                        │
│    ├── Convert AC to Telegram format:                    │
│    │   ├── ac_to_telegram() → HTML + InlineKeyboard      │
│    │   ├── Choose method: sendMessage / sendPhoto /      │
│    │   │   sendMediaGroup                                │
│    │   └── Truncate: 4096 (text) or 1024 (caption)      │
│    ├── HTTP POST to Telegram Bot API:                    │
│    │   https://api.telegram.org/bot{token}/sendMessage   │
│    │   Body: { chat_id, text, parse_mode: "HTML",        │
│    │          reply_markup: { inline_keyboard: [...] } }  │
│    └── Return result with sent message IDs               │
│                                                          │
│    Output: { ok: true, message_ids: ["123"] }            │
└─────────────────────────────────────────────────────────┘
```

### Operator Invocation Code (Pseudocode)

```rust
// File: greentic-operator/src/cli.rs (demo send handler)

// Step 1: Load pack
let runner_host = DemoRunnerHost::new(&bundle_path)?;
let pack = runner_host.resolve_pack("messaging-telegram")?;

// Step 2: Build input
let message = DemoSendMessage {
    text: "Hello from Greentic",
    to: vec![Destination { id: "7951102355" }],
    metadata: load_card_if_provided(card_path),
};

// Step 3: render_plan
let plan = runner_host.invoke_op(pack, "render_plan",
    json!({ "v": 1, "message": message }))?;

// Step 4: encode
let payload = runner_host.invoke_op(pack, "encode",
    json!({ "v": 1, "message": message, "plan": plan }))?;

// Step 5: send_payload
let result = runner_host.invoke_op(pack, "send_payload",
    json!({ "v": 1, "payload": payload }))?;

println!("Sent: {:?}", result);
```

---

## 3. Inbound Message Flow (demo ingress)

### CLI Command

```bash
# Recommended: use --body with a file path
GREENTIC_ENV=dev greentic-operator demo ingress \
  --bundle /path/to/demo-bundle \
  --provider messaging-telegram \
  --body /tmp/telegram-webhook.json \
  --tenant default \
  --print all
```

### Full Pipeline

```
Webhook payload (JSON file)
  │
  ▼
┌─────────────────────────────────────────────────────────┐
│ 1. BUILD INGRESS REQUEST                                 │
│    Operator builds IngressRequestV1 {                    │
│      method: "POST",                                     │
│      path: "/",                                          │
│      headers: [["content-type", "application/json"]],    │
│      body: [123, 34, 117, ...],  ← raw bytes (Vec<u8>)  │
│      query: [],                                          │
│    }                                                     │
│                                                          │
│    ⚠ Note: body is a JSON array of numbers, NOT base64.  │
│    The provider's parse_operator_http_in() handles both  │
│    formats (body_b64 string OR body byte array).         │
└─────────────────────┬───────────────────────────────────┘
                      │
                      ▼
┌─────────────────────────────────────────────────────────┐
│ 2. INGEST HTTP (WASM invoke)                             │
│    op: "ingest_http"                                     │
│                                                          │
│    Provider does:                                        │
│    ├── parse_operator_http_in(input)                     │
│    │   ├── If body_b64 present → use directly            │
│    │   └── If body:[u8] array → base64-encode it         │
│    ├── Decode base64 body → JSON                         │
│    ├── Parse Telegram update:                            │
│    │   ├── Extract message.text                          │
│    │   ├── Extract message.from (sender ID, name)        │
│    │   ├── Extract message.chat.id (destination)         │
│    │   └── Handle callback_query if present              │
│    ├── Build ChannelMessageEnvelope:                     │
│    │   { channel: "telegram",                            │
│    │     from: { id: "7951102355", kind: "user" },       │
│    │     to: [{ id: "7951102355", kind: "chat" }],       │
│    │     text: "Hello",                                  │
│    │     metadata: { chat_id: "7951102355",              │
│    │                 universal: "true" } }                │
│    └── Return HttpOutV1:                                 │
│        { v: 1, status: 200, body: "...",                 │
│          events: [<envelope>] }                           │
└─────────────────────┬───────────────────────────────────┘
                      │
                      ▼
┌─────────────────────────────────────────────────────────┐
│ 3. OPERATOR PROCESSES RESPONSE                           │
│    ├── Parse HttpOutV1                                   │
│    ├── Extract events[] → ChannelMessageEnvelopes        │
│    ├── Print to stdout                                   │
│    └── If --send or --end-to-end:                        │
│        └── Chain into egress pipeline                    │
│            (render_plan → encode → send_payload)         │
└─────────────────────────────────────────────────────────┘
```

### Server Mode (demo start)

When running `demo start`, the operator starts an HTTP server that receives real webhooks:

```bash
# Terminal 1: Start server
GREENTIC_ENV=dev greentic-operator demo start \
  --bundle /path/to/demo-bundle \
  --cloudflared off \
  --skip-setup \
  --env dev

# Terminal 2: POST webhook
curl -s -X POST http://localhost:8080/v1/messaging/ingress/messaging-telegram/default/default \
  -H "Content-Type: application/json" \
  -d @/tmp/telegram-webhook.json
```

```
External Service (Telegram/Slack/Teams)
  │
  │ Webhook POST
  ▼
┌──────────────────────────────────────┐
│  Operator HTTP Gateway                │
│  localhost:8080                        │
│                                        │
│  Route: /v1/messaging/ingress/         │
│         {provider}/{tenant}/{team}     │
└──────────────┬───────────────────────┘
               │
               ▼
┌──────────────────────────────────────┐
│  Ingress Handler                      │
│  Build IngressRequestV1:              │
│    body: payload_bytes (Vec<u8>)      │
│  Invoke WASM: ingest_http             │
│  Extract ChannelMessageEnvelopes      │
└──────────────┬───────────────────────┘
               │
               ▼
┌──────────────────────────────────────┐
│  Dispatch Egress (background thread)  │
│  For each event:                      │
│    render_plan → encode → send_payload│
│                                       │
│  Or (with NATS): publish to           │
│  greentic.messaging.ingress.          │
│  {env}.{tenant}.{team}.{provider}     │
└──────────────────────────────────────┘
```

---

## 4. WIT Interface Contract

### What the Provider Must Export

```wit
// greentic:component/runtime@0.6.1 (CBOR encoding)
interface runtime {
  invoke: func(op: string, input: list<u8>) -> list<u8>;
}

// greentic:provider-schema-core/schema-core-api@1.0.0 (JSON encoding)
interface schema-core-api {
  describe: func() -> list<u8>;
  validate-config: func(config-json: list<u8>) -> list<u8>;
  healthcheck: func() -> list<u8>;
  invoke: func(op: string, input-json: list<u8>) -> list<u8>;
}
```

### What Operations Must Be Supported

| Operation | Input | Output | Purpose |
|-----------|-------|--------|---------|
| `render_plan` | RenderPlanInV1 | RenderPlanOutV1 | Determine rendering tier, downsample AC |
| `encode` | EncodeInV1 | EncodeOutV1 (bytes) | Consume `render_plan` output payload and build provider-specific HTTP payload |
| `send_payload` | SendPayloadInV1 | SendPayloadOutV1 | Make HTTP call to external API |
| `ingest_http` | HttpInV1 | HttpOutV1 | Parse incoming webhook |
| `send` | ChannelMessageV1 | SendResultV1 | Legacy direct send (deprecated) |
| `reply` | ReplyMessageV1 | SendResultV1 | Reply to specific message |
| `qa-spec` | QaSpecRequest | QaSpecResponse | Return setup questions |
| `apply-answers` | ApplyAnswersRequest | ApplyAnswersResponse | Apply setup answers |
| `i18n-keys` | I18nRequest | I18nResponse | Return localization keys |

### What the Host Provides to the Provider

```wit
// greentic:http/http-client@1.1.0
interface http-client {
  send-request: func(req: http-request) -> http-response;
}

// greentic:secrets/store@1.0.0
interface secrets-store {
  get-secret: func(key: string) -> result<string, string>;
}

// greentic:state/state-store@1.0.0
interface state-store {
  get: func(key: string) -> option<list<u8>>;
  set: func(key: string, value: list<u8>);
  delete: func(key: string);
}
```

---

## 5. Provider Internal Architecture

### Module Structure (every provider follows this)

```
components/messaging-provider-telegram/src/
├── lib.rs          ← WIT bindings + dispatch
├── ops.rs          ← Operation handlers (render_plan, encode, send_payload, ingest_http)
├── config.rs       ← Secret resolution + provider config struct
└── describe.rs     ← Provider metadata, QA specs, i18n keys
```

### lib.rs — Entry Point & Dispatch

```rust
// Exports BOTH interfaces
impl runtime::Guest for Component {
    fn invoke(op: String, input_cbor: Vec<u8>) -> Vec<u8> {
        // CBOR → JSON bridge for v0.6 runtime
        cbor_json_invoke_bridge(&op, &input_cbor, Some("send"), |op, input| {
            dispatch_json_invoke(op, input)
        })
    }
}

impl schema_core_api::Guest for Component {
    fn invoke(op: String, input_json: Vec<u8>) -> Vec<u8> {
        // Direct JSON for operator v0.4.x
        dispatch_json_invoke(&op, &input_json)
    }
}

fn dispatch_json_invoke(op: &str, input: &[u8]) -> Vec<u8> {
    match op {
        "render_plan"  => render_plan(input),
        "encode"       => encode_op(input),
        "send_payload" => send_payload(input),
        "ingest_http"  => ingest_http(input),
        "send"         => handle_send(input),
        "reply"        => handle_reply(input),
        "qa-spec"      => qa_spec_op(input),
        "apply-answers"=> apply_answers_op(input),
        "i18n-keys"    => i18n_keys_op(input),
        other => error_response(format!("unsupported op: {other}")),
    }
}
```

### ops.rs — Message Pipeline

```rust
// render_plan: Determine how to display the message
pub fn render_plan(input: &[u8]) -> Vec<u8> {
    let req: RenderPlanInV1 = parse(input);
    let config = RenderPlanConfig {
        capabilities: PlannerCapabilities {
            supports_adaptive_card: false,  // Telegram can't render AC natively
            supports_markdown: true,
            supports_html: true,
            supports_images: false,
            supports_buttons: false,
            max_text_len: Some(4096),
        },
        default_summary: "New message",
    };
    render_plan_common(&req, &config)  // shared via provider-common
}

// encode: Build provider-specific payload
pub fn encode_op(input: &[u8]) -> Vec<u8> {
    let req: EncodeInV1 = parse(input);
    let mut envelope = build_channel_envelope(&req);

    // For non-AC providers: extract AC text at encode time
    if let Some(ac_json) = envelope.metadata.get("adaptive_card") {
        if let Some(summary) = extract_ac_summary(ac_json) {
            envelope.text = summary;
        }
    }
    serialize_payload(envelope)
}

// send_payload: Make HTTP call to Telegram API
pub fn send_payload(input: &[u8]) -> Vec<u8> {
    let req: SendPayloadInV1 = parse(input);
    let envelope: ChannelMessageEnvelope = deserialize(req.payload);
    let config = config_from_secrets();  // reads bot_token from host

    // Convert AC to Telegram HTML + inline keyboard
    let (text, reply_markup, images) = prepare_telegram_message(&envelope);

    // Call Telegram Bot API
    let url = format!("https://api.telegram.org/bot{}/sendMessage", config.bot_token);
    let body = json!({
        "chat_id": envelope.to,
        "text": text,
        "parse_mode": "HTML",
        "reply_markup": reply_markup,
    });

    let response = http_client::send_request(url, "POST", body);
    parse_telegram_response(response)
}
```

### config.rs — Secret Resolution

```rust
pub fn config_from_secrets() -> Result<ProviderConfig> {
    // Reads from host secrets store
    // Secret URI: secrets://dev/default/_/messaging-telegram/telegram_bot_token
    let bot_token = secrets_store::get_secret("telegram_bot_token")
        .or_else(|_| secrets_store::get_secret("bot_token"))?;  // fallback key

    Ok(ProviderConfig {
        bot_token,
        api_base_url: get_optional("api_base_url")
            .unwrap_or("https://api.telegram.org".into()),
        default_chat_id: get_optional("default_chat_id"),
        ..Default::default()
    })
}
```

---

## 6. Secrets Integration

### How Secrets Flow

```
Operator                          WASM Provider
────────                          ─────────────

.greentic/dev/                    config_from_secrets()
  .dev.secrets.env                    │
      │                               ▼
      │ load on startup           get_secret("telegram_bot_token")
      ▼                               │
DynSecretsManager ◄────────────────────┘
      │                           (via WIT import)
      ▼
Lookup: secrets://dev/default/_/
        messaging-telegram/
        telegram_bot_token
      │
      ▼
Return decrypted value
```

### Secret URI Format

```
secrets://{env}/{tenant}/{team}/{category}/{key}

Examples:
  secrets://dev/default/_/messaging-telegram/telegram_bot_token
  secrets://dev/default/_/messaging-slack/slack_bot_token
  secrets://dev/demo/_/messaging-teams/MS_GRAPH_REFRESH_TOKEN
  secrets://dev/demo/_/messaging-email/ms_graph_refresh_token
```

### Seeding Secrets (Demo Mode)

```bash
# Secrets are stored in encrypted dotenv format
# File: demo-bundle/.greentic/dev/.dev.secrets.env

# Seed via operator
GREENTIC_ENV=dev greentic-operator demo setup \
  --bundle /path/to/demo-bundle \
  --provider messaging-telegram \
  --setup-input '{"telegram_bot_token":"123:ABC..."}'

# Or manually via greentic-secrets CLI
greentic-secrets set \
  --env dev --tenant default --team _ \
  --category messaging-telegram \
  --key telegram_bot_token \
  --value "123:ABC..."
```

---

## 7. Adaptive Card Rendering Tiers

### How Tier Selection Works

```
render_plan_common()
    │
    ├── Parse AC JSON from metadata["adaptive_card"]
    │
    ├── extract_planner_card(ac_json)
    │   └── Walk AC body, extract text blocks, images, actions
    │
    ├── plan_render(card, capabilities)
    │   │
    │   ├── capabilities.supports_adaptive_card == true?
    │   │   ├── YES → TierA (pass through full AC JSON)
    │   │   └── NO  → TierD (downsample to text)
    │   │
    │   ├── Has attachments + supports_images?
    │   │   └── TierB (AC as attachment + fallback)
    │   │
    │   └── Apply max_text_len truncation
    │
    └── Return { tier, summary_text, warnings, attachments }
```

### Per-Provider Capabilities

| Provider | supports_ac | supports_md | supports_html | supports_img | supports_btn | max_text |
|----------|:-----------:|:-----------:|:-------------:|:------------:|:------------:|:--------:|
| Slack | false | true | false | false | false | 40,000 |
| Teams | **true** | true | false | true | true | - |
| Webex | **true** | true | true | true | false | - |
| Telegram | false | true | true | false | false | 4,096 |
| WhatsApp | false | false | false | false | false | 4,096 |
| Email | false | false | true | true | false | - |
| WebChat | **true** | true | true | true | true | - |

### What Each Tier Produces

**TierA** (Teams, WebChat): Full AC JSON passed through
```json
{ "tier": "A", "attachments": [{ "contentType": "application/vnd.microsoft.card.adaptive", "content": {...} }] }
```

**TierB** (Webex): AC as attachment + text fallback
```json
{ "tier": "B", "summary_text": "Meeting Reminder\n...", "attachments": [{ "contentType": "application/vnd.microsoft.card.adaptive", "content": {...} }] }
```

**TierD** (Slack, Telegram, WhatsApp, Email): Downsampled text
```json
{ "tier": "D", "summary_text": "Meeting Reminder\nPlease review...", "warnings": ["adaptive_card_downsampled"] }
```

---

## 8. NATS Integration (Production & Demo Start)

### Subjects

```
Ingress:  greentic.messaging.ingress.{env}.{tenant}.{team}.{provider}
Egress:   greentic.messaging.egress.{env}.{tenant}.{team}.{provider}

Example:
  greentic.messaging.ingress.dev.default.default.telegram
  greentic.messaging.egress.dev.default.default.telegram
```

### Flow in demo start Mode

```
External Webhook
      │
      ▼
┌─────────────────┐    NATS publish     ┌─────────────────┐
│ HTTP Gateway     │ ──────────────────► │ Ingress Subject  │
│ :8080            │                     │ greentic.msg.    │
│                  │                     │ ingress.dev.*    │
└─────────────────┘                     └────────┬────────┘
                                                  │
                                          NATS subscribe
                                                  │
                                                  ▼
                                        ┌─────────────────┐
                                        │ Message Router   │
                                        │ (App Flow)       │
                                        └────────┬────────┘
                                                  │
                                          NATS publish
                                                  │
                                                  ▼
                                        ┌─────────────────┐
                                        │ Egress Subject   │
                                        │ greentic.msg.    │
                                        │ egress.dev.*     │
                                        └────────┬────────┘
                                                  │
                                          NATS subscribe
                                                  │
                                                  ▼
                                        ┌─────────────────┐
                                        │ Egress Dispatcher│
                                        │ render_plan →    │
                                        │ encode →         │
                                        │ send_payload     │
                                        └─────────────────┘
```

### demo send vs demo ingress vs demo start

| Feature | `demo send` | `demo ingress` | `demo start` |
|---------|-------------|----------------|--------------|
| Execution | One-shot CLI | One-shot CLI | Long-running server |
| Direction | Egress only | Ingress (+ optional egress) | Both |
| Pipeline | render_plan → encode → send_payload | ingest_http (+ `--send` for egress) | Full bidirectional |
| Input | `--text` / `--card` | `--body <file>` (webhook JSON) | Real HTTP webhooks |
| NATS | Not used | Not used | Off by default, optional |
| Use case | Test egress pipeline | Test ingress parsing | Local development / demo |

---

## 9. Integration Contract: Operator → Provider Format Bridging

The operator and providers use slightly different serialization formats for HTTP ingress payloads. The provider-side `parse_operator_http_in()` (in `provider-common/src/http_compat.rs`) bridges these differences so both formats are accepted.

### Body Field

| Source | Field | Format | Example |
|--------|-------|--------|---------|
| Operator (`IngressRequestV1`) | `body` | JSON array of u8 numbers | `[123, 34, 117, ...]` |
| Provider (`HttpInV1`) | `body_b64` | Base64-encoded string | `"eyJ1cGRhdGVfaWQiOi..."` |

The provider accepts **both**. If `body_b64` is present, it's used directly. Otherwise, `body` (byte array) is base64-encoded on the fly. This was a bug fix — previously, an empty `body_b64` caused the provider to parse an empty body, losing all extracted fields (chat_id, from, text), which made egress fail with `"destination required"`.

### Headers Field

| Source | Format | Example |
|--------|--------|---------|
| Operator | Tuple arrays | `[["content-type", "application/json"]]` |
| Provider (`HttpInV1`) | Object array | `[{"name": "content-type", "value": "application/json"}]` |

Both formats are accepted by `parse_operator_http_in()`.

### Query Field

| Source | Format | Example |
|--------|--------|---------|
| Operator | Tuple arrays | `[["hub.mode", "subscribe"], ["hub.challenge", "abc"]]` |
| Provider (`HttpInV1`) | Query string | `"hub.mode=subscribe&hub.challenge=abc"` |

Tuple arrays are joined into a query string by `parse_operator_http_in()`.

### Response Format (Provider → Operator)

Providers return `HttpOutV1` with an injected `"v": 1` field for operator v0.4.x compatibility:

```json
{
  "v": 1,
  "status": 200,
  "headers": [["content-type", "application/json"]],
  "body_b64": "...",
  "events": [{ /* ChannelMessageEnvelope */ }]
}
```

Note: headers in the response are also serialized as tuple arrays (not objects) for operator compatibility.

### Summary

All format bridging is handled by `provider-common`. Individual providers call `parse_operator_http_in()` and `http_out_v1_bytes()` — they don't need to worry about format differences. If the operator changes its serialization format in the future, only `http_compat.rs` needs updating.

---

## 10. Error Handling

### Provider Errors

Providers return errors in a standard format:

```json
{
  "ok": false,
  "error": "Telegram API returned 401: Unauthorized",
  "retryable": false
}
```

### Operator Error Enrichment

The operator enriches errors with context:

```
If error contains "secret store error":
  → Print: "Missing secret: telegram_bot_token"
  → Print: "Seed it with: greentic-operator demo setup --provider messaging-telegram"

If error contains API failure:
  → Print: HTTP status + response body
  → Suggest: check bot token, API permissions

If error contains WASM trap:
  → Print: stack trace
  → Suggest: check WASM component compatibility
```

---

## 11. Adding a New Provider

To add a new messaging provider, follow this pattern:

### Step 1: Create Component

```
components/messaging-provider-newservice/
├── Cargo.toml          ← depends on provider-common, greentic-interfaces-guest
├── src/
│   ├── lib.rs          ← WIT bindings, dispatch_json_invoke()
│   ├── ops.rs          ← render_plan, encode, send_payload, ingest_http
│   ├── config.rs       ← Secret keys for the new service
│   └── describe.rs     ← Provider metadata, QA spec, i18n
└── wit/                ← Symlink to ../../wit
```

### Step 2: Implement Operations

```rust
// ops.rs - minimum required
pub fn render_plan(input: &[u8]) -> Vec<u8> {
    render_plan_common(&parse(input), &RenderPlanConfig {
        capabilities: PlannerCapabilities {
            supports_adaptive_card: false,  // or true
            supports_markdown: true,
            supports_html: false,
            max_text_len: Some(10000),
            ..Default::default()
        },
        default_summary: "New message",
    })
}

pub fn encode_op(input: &[u8]) -> Vec<u8> { /* build envelope */ }
pub fn send_payload(input: &[u8]) -> Vec<u8> { /* call API */ }
pub fn ingest_http(input: &[u8]) -> Vec<u8> { /* parse webhook */ }
```

### Step 3: Create Pack

```
packs/messaging-newservice/
├── pack.manifest.json
├── pack.yaml
├── flows/
│   ├── setup_default.ygtc
│   ├── render_plan.ygtc
│   ├── encode.ygtc
│   └── send_payload.ygtc
└── assets/
    └── setup.yaml
```

### Step 4: Build & Test

```bash
# Build WASM
cargo build --target wasm32-wasip2 --release -p messaging-provider-newservice

# Run tests
cargo test -p messaging-provider-newservice

# Test with operator
GREENTIC_ENV=dev greentic-operator demo send \
  --bundle demo-bundle \
  --provider messaging-newservice \
  --to "destination" \
  --text "Hello"
```
