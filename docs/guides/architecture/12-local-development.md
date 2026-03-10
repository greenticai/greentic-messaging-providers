# Local Development Guide

Cara setup dan running development di local untuk messaging providers.

---

## One-Time Setup

### 1. Install Prerequisites

```bash
# Rust toolchain
rustup target add wasm32-wasip2

# cargo tools
cargo install cargo-binstall --locked
cargo install cargo-component --locked

# greentic CLI tools
cargo binstall greentic-operator@0.4.23 --force --no-confirm
cargo binstall greentic-pack --force --no-confirm --locked
cargo binstall greentic-flow --force --no-confirm --locked
cargo binstall greentic-component --force --no-confirm --locked

# oras (untuk OCI - optional, hanya kalau perlu sync packs)
ORAS_VERSION="1.2.2"
curl -fsSLO "https://github.com/oras-project/oras/releases/download/v${ORAS_VERSION}/oras_${ORAS_VERSION}_linux_amd64.tar.gz"
tar -xzf "oras_${ORAS_VERSION}_linux_amd64.tar.gz" -C /usr/local/bin oras
rm -f "oras_${ORAS_VERSION}_linux_amd64.tar.gz"
```

### 2. Create Demo Bundle (sekali saja)

```bash
cd /root/works/personal/greentic

# Create bundle scaffold
gtc op demo new demo-bundle

# Copy pre-built packs
mkdir -p demo-bundle/providers/messaging
cp greentic-messaging-providers/dist/packs/messaging-telegram.gtpack demo-bundle/providers/messaging/
cp greentic-messaging-providers/dist/packs/messaging-webex.gtpack demo-bundle/providers/messaging/
cp greentic-messaging-providers/dist/packs/messaging-webchat.gtpack demo-bundle/providers/messaging/
cp greentic-messaging-providers/dist/packs/messaging-dummy.gtpack demo-bundle/providers/messaging/
```

### 3. Setup Input File

Buat `demo-bundle/setup-input.json`:
```json
{
  "messaging-telegram": {
    "public_base_url": "https://your-tunnel.trycloudflare.com",
    "default_chat_id": "",
    "bot_token": "YOUR_REAL_BOT_TOKEN"
  },
  "messaging-webex": {
    "public_base_url": "https://your-tunnel.trycloudflare.com",
    "bot_token": "YOUR_REAL_WEBEX_TOKEN"
  },
  "messaging-webchat": {
    "public_base_url": "https://your-tunnel.trycloudflare.com"
  },
  "messaging-dummy": {}
}
```

---

## Daily Development Workflow

### Quick Test: Send Message (tanpa setup/start)

Cara paling cepat untuk test provider setelah edit code:

```bash
# 1. Edit provider code
vim greentic-messaging-providers/components/messaging-provider-telegram/src/lib.rs

# 2. Rebuild WASM (hanya provider yang diubah)
cd greentic-messaging-providers
./tools/build_components/messaging-provider-telegram.sh

# 3. Update WASM di .gtpack
mkdir -p /tmp/pack-update/components/messaging-provider-telegram
cp target/components/messaging-provider-telegram/component.wasm /tmp/pack-update/components/messaging-provider-telegram/
cd /tmp/pack-update
zip -u /root/works/personal/greentic/demo-bundle/providers/messaging/messaging-telegram.gtpack \
  components/messaging-provider-telegram/component.wasm

# 4. Test
cd /root/works/personal/greentic
GREENTIC_ENV=dev gtc op demo send \
  --bundle demo-bundle \
  --provider messaging-telegram \
  --text "Hello test" \
  --tenant default \
  --env dev
```

### Shortcut Script

Simpan sebagai `dev-test.sh`:
```bash
#!/bin/bash
set -euo pipefail

PROVIDER="${1:-messaging-provider-telegram}"
PACK="${2:-messaging-telegram}"
TEXT="${3:-Hello test}"
ROOT="/root/works/personal/greentic"
BUNDLE="$ROOT/demo-bundle"
PROVIDERS_DIR="$ROOT/greentic-messaging-providers"

echo "==> Rebuilding $PROVIDER..."
cd "$PROVIDERS_DIR"
./tools/build_components/${PROVIDER}.sh

echo "==> Updating .gtpack..."
mkdir -p /tmp/pack-update/components/${PROVIDER}
cp "target/components/${PROVIDER}/component.wasm" /tmp/pack-update/components/${PROVIDER}/
cd /tmp/pack-update
zip -u "$BUNDLE/providers/messaging/${PACK}.gtpack" "components/${PROVIDER}/component.wasm"

echo "==> Sending test message..."
cd "$ROOT"
GREENTIC_ENV=dev gtc op demo send \
  --bundle "$BUNDLE" \
  --provider "$PACK" \
  --text "$TEXT" \
  --tenant default \
  --env dev \
  --debug

echo "==> Done!"
```

Pemakaian:
```bash
# Test telegram
./dev-test.sh messaging-provider-telegram messaging-telegram "Hello from telegram"

# Test webex
./dev-test.sh messaging-provider-webex messaging-webex "Hello from webex"

# Test dummy
./dev-test.sh messaging-provider-dummy messaging-dummy "Hello from dummy"
```

---

## Build Commands Reference

### Build Satu Provider WASM
```bash
cd greentic-messaging-providers
./tools/build_components/messaging-provider-telegram.sh
# Output: target/components/messaging-provider-telegram/component.wasm
```

### Build Semua 23 WASM Components
```bash
cd greentic-messaging-providers
SKIP_WASM_TOOLS_VALIDATION=1 ./tools/build_components.sh
# Output: target/components/*.wasm (23 files)
# Waktu: ~5-10 menit pertama kali, lebih cepat setelahnya (incremental)
```

### Build Operator dari Source
```bash
cd greentic-operator
cargo build --release
# Output: target/release/greentic-operator
# CATATAN: Versi source mungkin punya interface mismatch dengan packs.
# Lebih aman pakai published version: cargo binstall greentic-operator@0.4.23
```

### Sync Packs (setelah build components)
```bash
cd greentic-messaging-providers

# Pastikan templates ada di tempat yang benar
cp components/templates/templates.wasm target/components/ai.greentic.component-templates.wasm

# Sync
./tools/sync_packs.sh
```

---

## Operator Demo Commands

### demo send - Test Provider Pipeline

```bash
# Basic text message
GREENTIC_ENV=dev gtc op demo send \
  --bundle demo-bundle \
  --provider messaging-telegram \
  --text "Hello" \
  --tenant default \
  --env dev

# Dengan destination (untuk real send)
GREENTIC_ENV=dev gtc op demo send \
  --bundle demo-bundle \
  --provider messaging-telegram \
  --text "Hello" \
  --to "123456789" \
  --to-kind "chat" \
  --tenant default \
  --env dev

# Dengan adaptive card
GREENTIC_ENV=dev gtc op demo send \
  --bundle demo-bundle \
  --provider messaging-telegram \
  --card path/to/card.json \
  --tenant default \
  --env dev

# Print required args (lihat apa yang provider butuhkan)
GREENTIC_ENV=dev gtc op demo send \
  --bundle demo-bundle \
  --provider messaging-telegram \
  --print-required-args \
  --tenant default \
  --env dev
```

### demo doctor - Validate Bundle
```bash
GREENTIC_ENV=dev gtc op demo doctor \
  --bundle demo-bundle
```

### demo list-packs - List Detected Packs
```bash
GREENTIC_ENV=dev gtc op demo list-packs \
  --bundle demo-bundle
```

### demo list-flows - List Flows in a Pack
```bash
GREENTIC_ENV=dev gtc op demo list-flows \
  --bundle demo-bundle \
  --pack messaging-telegram
```

---

## Environment Variables

| Variable | Value | Keterangan |
|----------|-------|------------|
| `GREENTIC_ENV` | `dev` | **WAJIB** - secrets backend butuh dev/test env |
| `RUST_LOG` | `debug` | Optional - verbose logging |
| `SKIP_WASM_TOOLS_VALIDATION` | `1` | Optional - skip wasm-tools validation saat build |

---

## Project Structure

```
/root/works/personal/greentic/
├── greentic-messaging-providers/     # MAIN: provider code
│   ├── components/                   # Rust source per provider
│   │   ├── messaging-provider-telegram/src/lib.rs
│   │   ├── messaging-provider-webex/src/lib.rs
│   │   ├── messaging-provider-webchat/src/lib.rs
│   │   └── ...
│   ├── target/components/            # Built WASM outputs
│   ├── dist/packs/                   # Pre-built .gtpack archives
│   ├── packs/                        # Pack definitions (pack.yaml, flows, schemas)
│   ├── specs/providers/              # Provider spec YAML files
│   └── tools/                        # Build scripts
│
├── greentic-operator/                # Operator CLI
│   ├── target/release/greentic-operator
│   └── src/
│
├── demo-bundle/                      # Demo bundle (created by demo new)
│   ├── greentic.demo.yaml
│   ├── setup-input.json
│   ├── providers/messaging/          # .gtpack files here
│   ├── state/                        # Runtime state (auto-created)
│   └── tenants/
│
└── docs/                             # Our documentation
```

---

## Known Limitations

### 1. demo setup Tidak Jalan
Flow engine WASMs (templates-based) punya version mismatch. Gunakan `demo send` untuk test provider ops langsung.

### 2. Webchat Provider Fails
State-store interface mismatch. Perlu fix di webchat component atau upgrade interface version.

### 3. Pack Rebuild Blocked
greentic-pack CLI tidak bisa build .gtpack dari source karena state-store mismatch. Workaround: update individual WASMs di existing .gtpack pakai `zip -u`.

### 4. Incremental Updates Only
Karena tidak bisa rebuild .gtpack dari nol, kita hanya bisa update individual WASM files di .gtpack yang sudah ada. Kalau perlu ubah flow, schema, atau manifest, perlu fix pack build issue dulu.

---

## Tips

1. **Selalu pakai `GREENTIC_ENV=dev`** - tanpa ini secrets backend error
2. **Build incremental** - hanya rebuild provider yang diubah, jangan semua
3. **Test dengan dummy dulu** - paling simple, tanpa secrets/destination
4. **Pakai `--debug` flag** - untuk lihat detail request/response di pipeline
5. **Pre-push: jalankan `ci/local_check.sh`** - tapi skip kalau hanya testing lokal
