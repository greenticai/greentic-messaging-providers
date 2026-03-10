# Secrets Deep Dive

How the Greentic secrets system works: URI scheme, encryption, backends, the DEK cache bug, and how to seed secrets correctly.

## URI Format

```
secrets://{env}/{tenant}/{team}/{category}/{name}[@version]
```

| Segment | Rules | Example |
|---------|-------|---------|
| `env` | Lowercase alphanumeric + `-_.` | `dev`, `staging`, `prod` |
| `tenant` | Same rules | `default`, `acme` |
| `team` | Same rules, or `_` for wildcard | `_`, `payments`, `engineering` |
| `category` | Same rules, usually = provider pack ID | `messaging-telegram`, `messaging-slack` |
| `name` | Same rules | `telegram_bot_token`, `api_key` |
| `@version` | Optional semver suffix | `@v1` (rarely used) |

The `_` team placeholder means "all teams" — a secret scoped to `_` is readable by any team within that tenant.

Examples:

```
secrets://dev/default/_/messaging-telegram/telegram_bot_token
secrets://dev/default/_/messaging-webchat/jwt_signing_key
secrets://prod/acme/payments/kv/stripe_secret_key@v2
```

## Encryption Architecture

### Envelope encryption (two-layer)

```
Plaintext secret
    |
    v
HKDF-SHA256 (DEK + salt + secret_uri as info)
    |
    v
Derived key (unique per secret, from shared DEK)
    |
    v
AES-256-GCM encrypt (12-byte random nonce)
    |
    v
Stored: (ciphertext, nonce, hkdf_salt, wrapped_dek, algorithm)
```

Each secret gets a **unique derived key** via HKDF, using the secret's full URI as the `info` parameter. This means even though multiple secrets share the same DEK, each one has a distinct encryption key.

### Key hierarchy

```
Master key (per-backend)
    |
    v  wrap/unwrap
DEK (Data Encryption Key) — random 32 bytes, cached per category scope
    |
    v  HKDF derive
Per-secret key — deterministic from (DEK + salt + URI)
    |
    v  AES-256-GCM
Ciphertext
```

### Supported algorithms

| Algorithm | Nonce size | Notes |
|-----------|-----------|-------|
| AES-256-GCM | 12 bytes | Default |
| XChaCha20-Poly1305 | 24 bytes | Alternative |

Configurable via `SECRETS_ENC_ALGO` env var.

## The DEK Cache Bug

### How the cache works

The DEK cache stores one DEK per **scope + category**:

```
CacheKey = (env, tenant, team, category)
```

Note: the secret **name** is NOT part of the cache key. All secrets in the same category share one DEK within a session.

- LRU capacity: 256 entries
- TTL: 300 seconds (configurable via `SECRETS_DEK_CACHE_TTL_SECS`)

### The bug

When you seed secrets in **separate processes**:

```
Session 1:  seed telegram_bot_token
            → generate DEK-A, cache as (dev, default, _, messaging-telegram) → DEK-A
            → encrypt token with DEK-A, store

Session 2:  seed public_base_url
            → generate DEK-B (different!), cache as (dev, default, _, messaging-telegram) → DEK-B
            → encrypt url with DEK-B, store

Session 2:  read telegram_bot_token
            → cache has DEK-B (not DEK-A)
            → try to decrypt with DEK-B → FAIL (was encrypted with DEK-A)
```

The DEK stored with each secret (`wrapped_dek` in the envelope) is per-session. When you write in session 1, the wrapped DEK for `telegram_bot_token` is DEK-A. When session 2 writes `public_base_url`, it creates DEK-B. Now reading `telegram_bot_token` in session 2 would use DEK-B (from cache), but the stored wrapped_dek is for DEK-A.

Actually, the system **does** store the wrapped DEK per secret — so reading back uses the stored wrapped_dek to unwrap the correct DEK. The real issue is more subtle: the **DevStore** backend (file-based) serializes its entire state on each write. When session 2 writes, it loads the state (which includes session 1's secret), but the in-memory DEK cache starts empty. If the backend re-wraps during serialization, different sessions' KEK derivation can conflict.

### The fix: batch seeding

**Always seed all secrets for the same category in one invocation:**

```bash
# CORRECT: one invocation, one DEK session
seed-secret demo-bundle/.greentic/dev/.dev.secrets.env \
  "secrets://dev/default/_/messaging-telegram/telegram_bot_token" "1234567890:AAF..." \
  "secrets://dev/default/_/messaging-telegram/public_base_url" "https://tunnel.trycloudflare.com"
```

```bash
# WRONG: separate invocations, different DEK sessions
seed-secret store.env "secrets://dev/default/_/messaging-telegram/telegram_bot_token" "token"
seed-secret store.env "secrets://dev/default/_/messaging-telegram/public_base_url" "url"
```

The onboard QA API (`/api/onboard/qa/submit`) handles this correctly — it seeds all answers for a provider in a single write operation.

### If you hit the bug

Symptoms:
- `secret error: not-found` or decryption errors when reading a secret that was seeded successfully
- Works for the last-seeded secret but fails for earlier ones in the same category

Fix:

```bash
# Delete the corrupted store and re-seed everything in one batch
rm demo-bundle/.greentic/dev/.dev.secrets.env

seed-secret demo-bundle/.greentic/dev/.dev.secrets.env \
  "secrets://dev/default/_/messaging-telegram/telegram_bot_token" "token" \
  "secrets://dev/default/_/messaging-telegram/public_base_url" "url" \
  "secrets://dev/default/_/messaging-webchat/jwt_signing_key" "key" \
  "secrets://dev/default/_/messaging-slack/slack_bot_token" "xoxb-..."
```

Different categories (e.g. `messaging-telegram` vs `messaging-webchat`) can be in separate invocations — the bug only affects secrets within the **same category**.

## Backends

### DevStore (local development)

Used when `GREENTIC_ENV=dev`. File-based, single `.env` file.

**File format:** `demo-bundle/.greentic/dev/.dev.secrets.env`

```
SECRETS_BACKEND_STATE=<base64-encoded-json>
```

The base64 decodes to:

```json
{
  "secrets": [
    {
      "key": "secrets://dev/default/_/messaging-telegram/telegram_bot_token",
      "versions": [
        {
          "version": 1,
          "deleted": false,
          "record": {
            "meta": {
              "uri": "secrets://dev/default/_/messaging-telegram/telegram_bot_token",
              "visibility": "team",
              "contentType": "text"
            },
            "value": [98, 102, 55, ...],
            "envelope": {
              "algorithm": "aes256gcm",
              "nonce": [12, 98, 34, 55, 211, 7, 99, 12, 44, 55, 88, 21],
              "hkdf_salt": [32, 88, ...],
              "wrapped_dek": [32, 77, ...]
            }
          }
        }
      ]
    }
  ]
}
```

Key points:
- Plaintext never touches disk — `value` is the ciphertext bytes
- Master key derived from `GREENTIC_DEV_MASTER_KEY` env var (default: empty string → SHA256 of empty)
- DEK wrapping: XOR-based (not cryptographically strong, dev-only)
- File locking: uses `fs2` exclusive locks during read/write

### Cloud backends (production)

| Backend | Key wrapping | Storage |
|---------|-------------|---------|
| AWS Secrets Manager | KMS envelope | AWS Secrets Manager |
| Azure Key Vault | Azure Key Vault keys | Azure Key Vault secrets |
| GCP Secret Manager | Cloud KMS | GCP Secret Manager |
| Kubernetes | K8s secrets | K8s Secret objects |
| HashiCorp Vault | Vault Transit | Vault KV |

All use proper cryptographic key wrapping (not XOR).

## seed-secret Tool

### Build

```bash
cargo build --manifest-path tools/seed-secret/Cargo.toml --release
# Binary: tools/seed-secret/target/release/seed-secret
```

### Write secrets (batch)

```bash
seed-secret <store-path> <uri1> <val1> [<uri2> <val2> ...]
```

```bash
# Single secret
seed-secret demo-bundle/.greentic/dev/.dev.secrets.env \
  "secrets://dev/default/_/messaging-webchat/jwt_signing_key" "my-signing-key"

# Batch (all same category — REQUIRED to avoid DEK bug)
seed-secret demo-bundle/.greentic/dev/.dev.secrets.env \
  "secrets://dev/default/_/messaging-telegram/telegram_bot_token" "123:AAF..." \
  "secrets://dev/default/_/messaging-telegram/public_base_url" "https://tunnel.example.com"
```

After writing, the tool verifies each secret by reading it back and printing a preview (first 20 characters).

### Read a secret

```bash
seed-secret read demo-bundle/.greentic/dev/.dev.secrets.env \
  "secrets://dev/default/_/messaging-telegram/telegram_bot_token"
```

## How the Operator Reads Secrets

### WASM component access

WASM provider components read secrets via the WIT `secrets-store` interface. The operator's `DemoRunnerHost` provides this interface by:

1. Creating a `DevStore` from the `.dev.secrets.env` file path
2. Wrapping it in `SecretsManagerHandle`
3. Linking it to the Wasmtime WASM runtime

When a component calls `secrets_store::get("telegram_bot_token")`, the operator:

1. Builds the full URI: `secrets://{env}/{tenant}/{team}/{category}/{key}`
2. Reads from DevStore
3. Decrypts using the stored envelope (wrapped_dek → unwrap → HKDF derive → AES decrypt)
4. Returns plaintext to the component

### Team wildcard fallback

If reading at `secrets://dev/demo/team-a/messaging-telegram/token` fails, the operator automatically retries with team `_`:

```
First try:  secrets://dev/demo/team-a/messaging-telegram/token  → not found
Fallback:   secrets://dev/demo/_/messaging-telegram/token       → found!
```

This is implemented in `secrets_gate.rs` and allows secrets seeded at tenant level (`_`) to be readable by all teams.

## Environment Variables

| Variable | Purpose | Default |
|----------|---------|---------|
| `GREENTIC_ENV` | Environment name (used in URI `env` segment) | required |
| `GREENTIC_DEV_SECRETS_PATH` | DevStore file location | `.dev.secrets.env` |
| `GREENTIC_DEV_MASTER_KEY` | DevKeyProvider master key seed | empty string |
| `SECRETS_DEK_CACHE_TTL_SECS` | DEK cache TTL in seconds | `300` |
| `SECRETS_ENC_ALGO` | Encryption algorithm (`aes256gcm` or `xchacha20poly1305`) | `aes256gcm` |
| `GREENTIC_ALLOW_ENV_SECRETS` | Allow fallback to env vars for secrets | `0` |

## Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| `secret error: not-found` | Secret not seeded, or wrong URI | Check URI segments match exactly |
| Decryption fails for some secrets | DEK cache bug (seeded in separate sessions) | Delete `.dev.secrets.env`, re-seed all in one batch |
| `secret error: not-found` with correct URI | Team mismatch | Seed with `_` team wildcard, or check team fallback |
| Works locally but not in operator | `GREENTIC_ENV` not set | Export `GREENTIC_ENV=dev` before running operator |
| Secret value is empty | Seeded with empty string | Re-seed with actual value |
| `SECRETS_BACKEND_STATE` corrupted | File was edited manually | Delete and re-seed from scratch |

## Quick Reference

```bash
# Seed all Telegram secrets (one batch)
seed-secret store.env \
  "secrets://dev/default/_/messaging-telegram/telegram_bot_token" "123:AAF..." \
  "secrets://dev/default/_/messaging-telegram/public_base_url" "https://tunnel.example.com"

# Seed WebChat secret
seed-secret store.env \
  "secrets://dev/default/_/messaging-webchat/jwt_signing_key" "my-key"

# Seed Slack secrets (one batch)
seed-secret store.env \
  "secrets://dev/default/_/messaging-slack/slack_bot_token" "xoxb-..." \
  "secrets://dev/default/_/messaging-slack/slack_app_id" "A0123..."

# Read back
seed-secret read store.env "secrets://dev/default/_/messaging-telegram/telegram_bot_token"

# Nuclear option: delete and re-seed everything
rm store.env
seed-secret store.env \
  "secrets://dev/default/_/messaging-telegram/telegram_bot_token" "123:AAF..." \
  "secrets://dev/default/_/messaging-telegram/public_base_url" "https://tunnel.example.com" \
  "secrets://dev/default/_/messaging-webchat/jwt_signing_key" "my-key" \
  "secrets://dev/default/_/messaging-slack/slack_bot_token" "xoxb-..."
```
