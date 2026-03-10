# Codebase Guide - Greentic Messaging Providers

Panduan untuk memahami codebase greentic-messaging-providers dari nol.

---

## 1. The Big Picture

```
Browser/App ──HTTP POST──> Operator (HTTP server, port 8080)
                               │
                               ├── Inbound (webhook masuk)
                               │   POST /ingress/messaging/{provider}/{tenant}
                               │   → operator panggil provider WASM op: "ingest_http"
                               │   → hasilnya: ChannelMessageEnvelope (pesan terstandar)
                               │
                               └── Outbound (kirim pesan)
                                   operator panggil 3 ops berurutan:
                                   1. render_plan  → "mau dikirim apa, format apa?"
                                   2. encode       → "konversi jadi payload siap kirim"
                                   3. send_payload → "kirim ke API external (Telegram/Webex/dll)"
```

**Kunci:** Semua provider adalah **WASM component** yang di-load oleh operator. Provider tidak jalan sendiri - operator yang panggil fungsi-fungsi di dalamnya.

---

## 2. Repo yang Kamu Kerjain

Dari 32 repo yang di-clone, kamu cuma perlu fokus ke **1 repo**:

```
greentic-messaging-providers/     ← INI REPO UTAMA KAMU
├── components/                   ← Source code Rust per provider
│   ├── messaging-provider-telegram/src/lib.rs
│   ├── messaging-provider-webex/src/lib.rs
│   ├── messaging-provider-webchat/src/lib.rs
│   ├── messaging-provider-dummy/src/lib.rs      ← referensi paling simple
│   ├── messaging-provider-slack/src/lib.rs
│   ├── messaging-provider-teams/src/lib.rs
│   ├── messaging-provider-whatsapp/src/lib.rs
│   ├── messaging-provider-email/src/lib.rs
│   ├── messaging-ingress-telegram/src/lib.rs    ← custom webhook handler
│   └── ...
├── crates/                       ← Shared libraries
│   ├── messaging-core/           ← Tipe-tipe shared (envelope, etc)
│   ├── provider-common/          ← Helper functions
│   └── provider-tests/           ← Test utilities
├── packs/                        ← Pack definitions (YAML + schemas)
│   ├── messaging-telegram/
│   ├── messaging-webex/
│   ├── messaging-webchat/
│   └── messaging-dummy/
├── specs/providers/              ← Provider spec files
├── tools/                        ← Build scripts
└── target/components/            ← Output WASM files (hasil build)
```

**greentic-operator** dipakai sebagai **binary tool** aja (`v0.4.23`, sudah di-install). Gak perlu edit.

---

## 3. Arsitektur: WASM Component Model

### Apa itu WASM Component?

Provider ditulis dalam Rust, tapi **tidak jalan sebagai binary biasa**. Dia di-compile jadi `.wasm` file yang di-load oleh operator pada runtime.

```
Rust code (.rs)
    │  cargo component build --target wasm32-wasip2
    ▼
WASM component (.wasm)
    │  di-bundle dalam .gtpack (ZIP archive)
    ▼
Operator load .gtpack → extract .wasm → jalankan fungsi-fungsi di dalamnya
```

### WIT Interface (WebAssembly Interface Types)

WIT mendefinisikan **kontrak** antara provider (guest) dan operator (host). Ibarat interface di Java/Go.

File: `wit/provider-core/world.wit`
```wit
package greentic:provider-schema-core@1.0.0;

interface schema-core-api {
  // Deskripsikan provider ini (ops apa aja, config schema, dll)
  describe: func() -> list<u8>;

  // Validasi config JSON
  validate-config: func(config-json: list<u8>) -> list<u8>;

  // Health check
  healthcheck: func() -> list<u8>;

  // Panggil operasi (send, render_plan, encode, dll)
  invoke: func(op: string, input-json: list<u8>) -> list<u8>;
}
```

**Semua provider export interface yang sama:** `schema-core-api`. Yang beda cuma **import**-nya:

| Provider | Import | Kenapa |
|----------|--------|--------|
| Telegram | `http/client`, `secrets-store` | Perlu HTTP ke Telegram API + baca bot token |
| Webex | `http/client`, `secrets-store` | Perlu HTTP ke Webex API + baca bot token |
| Webchat | `state-store`, `secrets-store` | Gak perlu HTTP, simpan data di state store |
| Dummy | (nothing) | Mock, gak perlu apa-apa |

Contoh WIT world untuk Telegram:
```wit
world messaging-provider-telegram {
    import greentic:http/client@1.1.0;        // bisa panggil HTTP
    import greentic:secrets-store@1.0.0;      // bisa baca secrets
    export greentic:provider-schema-core@1.0.0; // wajib implement ini
}
```

---

## 4. Anatomy of a Provider (Baca Dummy Dulu!)

Dummy adalah provider paling simple. Baca ini dulu sebelum yang lain.

File: `components/messaging-provider-dummy/src/lib.rs`

### 4.1 Boilerplate WIT Binding

```rust
// Generate Rust bindings dari WIT interface
wit_bindgen::generate!({
    world: "schema-core",
    // ... path ke WIT files
});

// Struct kosong sebagai "implementation target"
struct Component;

// Daftarkan struct ini sebagai export
export!(Component);
```

### 4.2 Implement Guest Trait

```rust
impl Guest for Component {
    fn describe() -> Vec<u8> { ... }
    fn validate_config(config_json: Vec<u8>) -> Vec<u8> { ... }
    fn healthcheck() -> Vec<u8> { ... }
    fn invoke(op: String, input_json: Vec<u8>) -> Vec<u8> { ... }
}
```

### 4.3 Operation Router (invoke)

`invoke()` adalah **pintu masuk utama**. Operator panggil ini dengan nama operasi:

```rust
fn invoke(op: String, input_json: Vec<u8>) -> Vec<u8> {
    match op.as_str() {
        "send"         => handle_send(&input_json),
        "reply"        => handle_reply(&input_json),
        "ingest_http"  => ingest_http(&input_json),
        "render_plan"  => render_plan(&input_json),
        "encode"       => encode_op(&input_json),
        "send_payload" => send_payload(&input_json),
        other => json_bytes(&json!({
            "ok": false,
            "error": format!("unsupported op: {other}")
        })),
    }
}
```

### 4.4 The 6 Operations

Setiap provider implement 6 ops (minimal). Ini penjelasannya:

#### `describe()` - Deskripsikan diri sendiri
```rust
fn describe() -> Vec<u8> {
    json_bytes(&json!({
        "provider_type": "messaging.dummy",
        "ops": ["send", "reply", "ingest_http", "render_plan", "encode", "send_payload"],
        "capabilities": [],
    }))
}
```

#### `render_plan(input)` - Phase 1: Rencanakan pengiriman
- **Input:** `RenderPlanInV1 { message }` - pesan yang mau dikirim
- **Output:** `RenderPlanOutV1 { ok, plan }` - rencana kirim (tier, format)
- **Fungsi:** Tentukan render tier (TierA-TierD) dan summary

```rust
fn render_plan(input: &[u8]) -> Vec<u8> {
    let parsed: Value = serde_json::from_slice(input).unwrap();
    let text = parsed["message"]["text"].as_str().unwrap_or("");
    json_bytes(&json!({
        "ok": true,
        "plan": {
            "render_tier": "tier-d",
            "summary": text,
        }
    }))
}
```

#### `encode(input)` - Phase 2: Encode jadi payload siap kirim
- **Input:** `EncodeInV1 { message, plan }` - pesan + rencana dari phase 1
- **Output:** `EncodeOutV1 { ok, payload }` - `ProviderPayloadV1` (base64 body)
- **Fungsi:** Convert message envelope → payload yang siap dikirim ke API

```rust
fn encode_op(input: &[u8]) -> Vec<u8> {
    // Ambil text dari message
    // Encode jadi ProviderPayloadV1
    json_bytes(&json!({
        "ok": true,
        "payload": {
            "content_type": "application/json",
            "body_b64": base64_encode(payload_bytes),
            "metadata": {}
        }
    }))
}
```

#### `send_payload(input)` - Phase 3: Kirim ke API external
- **Input:** `SendPayloadInV1 { payload, tenant }` - payload dari phase 2
- **Output:** `SendPayloadOutV1 { ok, message_id }` - hasil pengiriman
- **Fungsi:** Decode payload → kirim HTTP request ke API provider

```rust
fn send_payload(input: &[u8]) -> Vec<u8> {
    // Decode base64 body
    // Untuk Telegram: POST https://api.telegram.org/bot{token}/sendMessage
    // Untuk Dummy: just return ok
    json_bytes(&json!({
        "ok": true,
        "message_id": "generated-id",
    }))
}
```

#### `ingest_http(input)` - Terima webhook/inbound message
- **Input:** `HttpInV1 { method, path, headers, body_b64 }` - HTTP request masuk
- **Output:** `HttpOutV1 { status, headers, events }` - response + parsed events
- **Fungsi:** Parse webhook body → extract pesan → kembalikan sebagai event

#### `send(input)` / `reply(input)` - Legacy direct send
- Kirim pesan langsung (tanpa 3-phase pipeline)
- Masih dipakai tapi akan diganti oleh pipeline

---

## 5. Config dan Secrets

### Config

Setiap provider punya `ProviderConfig` struct:

```rust
// Telegram
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]   // <-- strict: tolak field yang gak dikenal
struct ProviderConfig {
    default_chat_id: Option<String>,
    api_base_url: Option<String>,    // default: https://api.telegram.org
}

// Webex
struct ProviderConfig {
    default_room_id: Option<String>,
    default_to_person_email: Option<String>,
    api_base_url: Option<String>,    // default: https://webexapis.com/v1
}

// Webchat
struct ProviderConfig {
    route: Option<String>,
    tenant_channel_id: Option<String>,
    public_base_url: Option<String>,
}
```

Config di-load dari JSON yang dikirim operator:
```rust
fn load_config(input: &Value) -> Result<ProviderConfig, String> {
    // Cek nested "config" key dulu
    if let Some(cfg) = input.get("config") {
        return serde_json::from_value(cfg.clone()).map_err(|e| e.to_string());
    }
    // Fallback ke root level
    serde_json::from_value(input.clone()).map_err(|e| e.to_string())
}
```

### Secrets

Provider akses secrets via WIT import:
```rust
use greentic::secrets_store::secrets_store;

// Baca secret
let token = secrets_store::get("TELEGRAM_BOT_TOKEN")
    .map_err(|e| format!("secrets error: {e}"))?
    .ok_or("bot token not found")?;
let token_str = String::from_utf8(token).unwrap();
```

Secrets di-declare di `component.manifest.json`:
```json
{
  "secret_requirements": [
    {
      "name": "TELEGRAM_BOT_TOKEN",
      "scope": "tenant",
      "description": "Telegram Bot API token"
    }
  ]
}
```

---

## 6. Message Flow Detail

### Outbound: Demo Send

```
gtc op demo send --provider messaging-telegram --text "Hello"
    │
    │  1. Build ChannelMessageEnvelope dari args
    ▼
invoke_provider_op("render_plan", envelope)
    │  Provider return: { ok: true, plan: { render_tier: "tier-d", summary: "Hello" } }
    ▼
invoke_provider_op("encode", { message: envelope, plan: plan })
    │  Provider return: { ok: true, payload: { content_type: "application/json",
    │                     body_b64: "<base64 encoded payload>", metadata: {} } }
    ▼
invoke_provider_op("send_payload", { payload: payload })
    │  Provider: decode base64 → POST ke Telegram API → return result
    ▼
{ ok: true/false, message_id: "tg:12345" }
```

### Inbound: HTTP Ingress

```
POST http://localhost:8080/ingress/messaging/messaging-telegram/default
  Body: { "update_id": 123, "message": { "text": "hello", "chat": { "id": 456 } } }
    │
    │  1. Parse URL: domain=messaging, provider=messaging-telegram, tenant=default
    │  2. Check supports_op("ingest_http") ← harus ada di entry_flows!
    │  3. Build HttpInV1 { method: "POST", path: "/...", body_b64: "<base64>" }
    ▼
invoke_provider_op("ingest_http", http_in)
    │  Provider: decode body → extract text/chat_id/user → build envelope
    │  Return: HttpOutV1 { status: 200, events: [ChannelMessageEnvelope] }
    ▼
Operator punya list of ChannelMessageEnvelope untuk diproses selanjutnya
```

### invoke_provider_op - Dua Jalur

Operator punya dua cara panggil ops:

```
invoke_provider_op(domain, provider, "render_plan", payload)
    │
    ├── op_id IN entry_flows?
    │   YES → jalankan lewat Flow Engine (template WASMs)
    │          cocok untuk: setup_default, diagnostics, verify_webhooks
    │
    └── NO → panggil WASM component langsung (invoke_provider_component_op)
             cocok untuk: render_plan, encode, send_payload
             INI YANG DIPAKAI SEKARANG untuk demo send
```

**Kenapa penting:** `render_plan`, `encode`, `send_payload` **TIDAK** ada di entry_flows manifest, jadi mereka bypass flow engine dan langsung panggil WASM. Ini sebabnya `demo send` jalan walaupun flow engine broken.

---

## 7. Pack System (.gtpack)

### Apa itu .gtpack?

File `.gtpack` adalah **ZIP archive** yang berisi:
```
messaging-telegram.gtpack (ZIP) — Capability-Driven Pattern
├── manifest.cbor              ← metadata pack (CBOR format)
├── components/
│   ├── messaging-provider-telegram/
│   │   └── component.wasm     ← provider WASM (qa-spec/apply-answers/i18n-keys built-in)
│   └── messaging-ingress-telegram/
│       └── component.wasm     ← ingress WASM
├── flows/
│   ├── setup_default.ygtc                  ← single-node: invoke messaging.configure
│   └── requirements.ygtc                   ← single-node: invoke messaging.configure
├── schemas/
│   └── messaging/telegram/
│       └── public.config.schema.json
└── assets/
    └── setup.yaml
```

### pack.yaml → manifest.cbor

`pack.yaml` adalah source definition. `greentic-pack` CLI compile ini jadi `manifest.cbor` di dalam `.gtpack`. Tapi karena greentic-pack CLI broken (state-store mismatch), kita pakai **pre-built .gtpack** dari `dist/packs/` dan update individual WASMs pakai `zip -u`.

### Entry Flows

`manifest.cbor` punya `meta.entry_flows` yang menentukan ops mana yang jalan lewat flow engine. Saat ini:
- `meta.entry_flows` = **not set** di semua pack
- Fallback ke flow IDs: `[diagnostics, requirements, setup_default, sync_subscriptions, verify_webhooks]`
- `render_plan`, `encode`, `send_payload` **TIDAK** ada di sini → langsung panggil WASM

---

## 8. Build & Test Workflow

### Build Satu Provider

```bash
cd greentic-messaging-providers

# Build WASM
./tools/build_components/messaging-provider-telegram.sh
# Output: target/components/messaging-provider-telegram/component.wasm

# Update di .gtpack
mkdir -p /tmp/pu/components/messaging-provider-telegram
cp target/components/messaging-provider-telegram/component.wasm /tmp/pu/components/messaging-provider-telegram/
cd /tmp/pu
zip -u /root/works/personal/greentic/demo-bundle/providers/messaging/messaging-telegram.gtpack \
  components/messaging-provider-telegram/component.wasm
```

### Test

```bash
cd /root/works/personal/greentic

# One-shot test (tanpa server)
GREENTIC_ENV=dev gtc op demo send \
  --bundle demo-bundle \
  --provider messaging-telegram \
  --text "Hello" \
  --tenant default --env dev

# Atau jalankan server
GREENTIC_ENV=dev gtc op demo start \
  --bundle demo-bundle \
  --skip-setup \
  --cloudflared off \
  --nats off \
  --tenant default --env dev

# Lalu test via curl
curl -X POST http://127.0.0.1:8080/ingress/messaging/messaging-telegram/default \
  -H "Content-Type: application/json" \
  -d '{"update_id":123,"message":{"text":"hello","chat":{"id":456}}}'
```

### Rust Unit Tests

```bash
cd greentic-messaging-providers
cargo test                    # test semua crates (bukan WASM components)
cargo test -p provider-tests  # test utilities aja
```

Note: WASM components sendiri gak bisa di-test via `cargo test` biasa. Test-nya lewat `demo send`.

---

## 9. Provider Comparison Cheat Sheet

| Aspek | Dummy | Telegram | Webex | Webchat |
|-------|-------|----------|-------|---------|
| **HTTP calls** | Tidak | Ya (Telegram API) | Ya (Webex API) | Tidak |
| **State store** | Tidak | Tidak | Tidak | Ya |
| **Secrets** | Tidak | TELEGRAM_BOT_TOKEN | WEBEX_BOT_TOKEN | jwt_signing_key* |
| **Ingress mode** | None | Custom (separate WASM) | Default (ingest_http) | Default (Direct Line) |
| **send_payload** | Mock OK | POST /sendMessage | POST /messages | Write to state store |
| **Config fields** | None | default_chat_id, api_base_url | default_room_id, default_to_person_email, api_base_url | route, tenant_channel_id, public_base_url |
| **Status** | Working | Working | Working (config fix needed) | Broken (state-store mismatch) |

*jwt_signing_key belum di-declare di manifest - ini salah satu bug yang perlu di-fix.

---

## 10. Known Issues yang Perlu Di-fix

### Priority 1: Quick Wins

| # | Issue | File | Fix |
|---|-------|------|-----|
| W1 | Webex `deny_unknown_fields` tolak `public_base_url` | `components/messaging-provider-webex/src/lib.rs:33` | Hapus `deny_unknown_fields` atau tambah field |
| C1 | Webchat `jwt_signing_key` gak di-declare | `components/messaging-provider-webchat/component.manifest.json` | Tambah ke `secret_requirements` |
| C2 | Webchat config schema salah (`mode`, `base_url` vs `public_base_url`) | `components/messaging-provider-webchat/schemas/.../config.schema.json` | Align dengan struct di code |

### Priority 2: Functional Issues

| # | Issue | Impact |
|---|-------|--------|
| Webchat state-store mismatch | Webchat gak bisa jalan sama sekali | Perlu align interface version (guest 0.4.89 vs host 0.4.93) |
| HTTP ingress 404 | Webhook gak bisa masuk | `ingest_http` gak ada di entry_flows |
| Hardcoded tenant "default" | Multi-tenant gak jalan | Webex & Webchat hardcode tenant |

### Priority 3: Demo Setup

| # | Issue | Impact |
|---|-------|--------|
| Flow engine templates.wasm | `demo setup` error | Templates compiled dengan interface version lama |

---

## 11. Urutan Baca Code (Recommended)

1. **`messaging-provider-dummy/src/lib.rs`** - Paling simple, pahami struktur dasar
2. **`messaging-provider-telegram/src/lib.rs`** - HTTP provider pattern, secrets access
3. **`messaging-provider-webex/src/lib.rs`** - Lebih complex (multiple dest types, adaptive cards)
4. **`messaging-provider-webchat/src/lib.rs`** - Model beda (state-store, Direct Line)
5. **`messaging-ingress-telegram/src/lib.rs`** - Custom ingress pattern (kalau perlu)

---

## 12. Glossary

| Term | Artinya |
|------|---------|
| **WIT** | WebAssembly Interface Types - kontrak antara host dan guest |
| **WASM Component** | Binary .wasm yang di-load oleh operator |
| **Guest** | Provider WASM (yang kamu tulis) |
| **Host** | Operator yang load dan jalankan guest |
| **Schema-core** | Interface standar yang semua provider implement |
| **Entry flows** | Daftar ops yang jalan lewat flow engine |
| **gtpack** | ZIP archive berisi WASM + manifest + flows + schemas |
| **Render tier** | Level rendering (TierA=basic text, TierD=full adaptive card) |
| **ChannelMessageEnvelope** | Struct standar untuk pesan (dari/ke semua channel) |
| **ProviderPayloadV1** | Payload ter-encode siap kirim (output dari encode op) |
| **Direct Line** | Microsoft Bot Framework protocol (dipakai webchat) |
| **CBOR** | Compact Binary Object Representation (format manifest) |
