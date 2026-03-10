# Troubleshooting Guide

Common issues encountered when running `greentic-operator` in demo mode with messaging providers. All entries are based on real problems hit during development and E2E testing.

---

## 1. Operator Startup Issues

### Port 8080 already in use

**Symptom:** `demo start` fails with "Address already in use (os error 98)".

**Cause:** A previous operator process or another service is holding port 8080.

**Fix:**

```bash
# Find and kill the process on port 8080
fuser -k 8080/tcp

# Or identify it first
ps aux | grep greentic-operator
kill <pid>
```

### Secrets backend error

**Symptom:** "failed to open dev store" or "secret not found" immediately on startup or during provider setup.

**Cause:** The operator's dev secrets backend requires `GREENTIC_ENV=dev` and expects the secrets file to exist on disk.

**Fix:**

```bash
# Ensure environment variable is set
export GREENTIC_ENV=dev

# Verify the secrets file exists
ls -la demo-bundle/.greentic/dev/.dev.secrets.env
```

If the file is missing, you need to re-seed secrets. See section 4 below.

---

## 2. Cloudflared Tunnel Issues

### Stale tunnel URL

**Symptom:** Operator logs reference an old tunnel URL. Webhooks from Telegram or Webex never arrive.

**Cause:** Cloudflared quick tunnels (`trycloudflare.com`) assign a new random URL on every restart. The URL from the previous session is no longer valid.

**Fix:**

```bash
# Get the current tunnel URL from logs
grep -oP 'https://[a-z0-9-]+\.trycloudflare\.com' demo-bundle/logs/cloudflared.log | tail -1
```

After retrieving the new URL, you must re-register webhooks for any provider that depends on them (Telegram, Webex). For Telegram:

```bash
curl "https://api.telegram.org/bot<TOKEN>/setWebhook?url=<NEW_TUNNEL_URL>/webhook/telegram"
```

### Tunnel not starting

**Symptom:** No tunnel URL appears in logs. Webhooks have no public endpoint.

**Diagnosis:**

```bash
# Check if cloudflared process is running
ps aux | grep cloudflared

# Check logs for errors
tail -20 demo-bundle/logs/cloudflared.log
```

**Fix:** If the tunnel is not needed (e.g., testing egress only), disable it:

```bash
gtc op demo start --cloudflared=off
```

---

## 3. WASM Build Cache Issues

### Stale WASM artifacts

**Symptom:** Provider operations fail with errors like:

- "invalid encode input: missing field `X`"
- Schema version mismatches
- Unexpected CBOR/JSON deserialization failures

**Cause:** The WASM component was compiled against an older version of `greentic-types` or `greentic-interfaces`. The binary format of invocation payloads has changed.

**Diagnosis:**

```bash
# Check which greentic-types version the WASM was built against
strings provider.wasm | grep greentic.types
```

**Fix:** Clean the WASM target directory and rebuild all providers from scratch:

```bash
cd /root/works/personal/greentic/greentic-messaging-providers
rm -rf target/wasm32-wasip2/
SKIP_WASM_TOOLS_VALIDATION=1 bash tools/build_components.sh
```

After rebuilding, update the gtpacks in your demo bundle. See section 7 for the correct `zip -u` procedure.

---

## 4. Secret Issues

### DEK Cache Bug (greentic-secrets-core v0.4.22)

**Symptom:** "MAC check failed" or "failed to decrypt" when reading secrets that were previously written successfully.

**Cause:** The `CacheKey` in secrets-core is `(env, tenant, team, category)` -- it does not include the secret name. All secrets within the same category share a single DEK cache slot. If secrets in the same category are written by separate processes, each process generates a different DEK and the second write's DEK overwrites the first in the cache, making the first secret undecryptable.

**Fix:** Write ALL secrets for a given category in a single process invocation. Use the `seed_all` binary for batch seeding:

```bash
# Good: one process writes all telegram secrets
./seed_all --category messaging-telegram \
  telegram_bot_token=<token> \
  bot_token=<token>

# Bad: separate invocations for the same category
./write_secret --category messaging-telegram --name telegram_bot_token ...
./write_secret --category messaging-telegram --name bot_token ...
# ^ Second call may corrupt the first secret's DEK
```

### Wrong tenant

**Symptom:** "secret not found" even though secrets were previously seeded.

**Cause:** Secrets were seeded under `tenant=default` but the operator command is using `tenant=demo` (or vice versa).

**Fix:** Either match the tenant on the CLI command:

```bash
gtc op demo send --tenant default --provider messaging-telegram ...
```

Or re-seed secrets under the correct tenant.

### Reading secrets for debugging

To verify a secret exists and can be decrypted:

```bash
./greentic-operator/target/release/read_secret \
  demo-bundle/.greentic/dev/.dev.secrets.env \
  "secrets://dev/default/_/messaging-telegram/telegram_bot_token"
```

---

## 5. Provider-Specific Issues

### Telegram

**`setWebhook` returns "bad webhook: Failed to resolve host"**

Cause: The cloudflared tunnel URL's DNS has not propagated yet.
Fix: Wait 1-2 minutes and retry. If the problem persists, restart cloudflared to get a new URL.

**`getWebhookInfo` shows `pending_update_count > 0`**

Cause: Stale updates queued from previous bot activity.
Fix: They will drain automatically over time. To clear immediately:

```bash
curl "https://api.telegram.org/bot<TOKEN>/deleteWebhook?drop_pending_updates=true"
# Then re-set the webhook
curl "https://api.telegram.org/bot<TOKEN>/setWebhook?url=<TUNNEL_URL>/webhook/telegram"
```

### Webex

**502 on ingress**

Cause: Webex webhooks send a message ID, and the ingress component makes a `GET /messages/{id}` call to the Webex API to fetch the actual content. With simulated or test data, the message ID does not correspond to a real Webex message, so the GET request fails.
Fix: Full ingress testing requires real Webex messages. There is no workaround for simulated data.

**ProviderConfig validation fails with "unknown field"**

Cause: The `ProviderConfig` struct uses `#[serde(deny_unknown_fields)]`. Passing `public_base_url` in config (which is valid in the schema) causes deserialization to fail.
Fix: Only include fields that the Rust struct accepts: `default_room_id`, `default_to_person_email`, `api_base_url`.

### WebChat

**"conversation not found" on `POST /conversations/{id}/activities`**

Cause: Using the initial token from `POST /token` instead of the conversation-bound token returned by `POST /v3/directline/conversations`.
Fix: Always use the token from the conversation creation response for subsequent activity posts:

```bash
# Step 1: Get initial token
TOKEN=$(curl -s -X POST http://localhost:8080/v3/directline/token | jq -r '.token')

# Step 2: Create conversation (returns a NEW token)
CONV=$(curl -s -X POST http://localhost:8080/v3/directline/conversations \
  -H "Authorization: Bearer $TOKEN")
CONV_TOKEN=$(echo $CONV | jq -r '.token')
CONV_ID=$(echo $CONV | jq -r '.conversationId')

# Step 3: Post activity using the CONVERSATION token, not the initial token
curl -X POST "http://localhost:8080/v3/directline/conversations/$CONV_ID/activities" \
  -H "Authorization: Bearer $CONV_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"type":"message","text":"hello","from":{"id":"user1"}}'
```

### Teams

**401 Unauthorized on send**

Cause: The OAuth refresh token has expired. Microsoft refresh tokens for public clients have a limited lifetime.
Fix: Re-authenticate via the OAuth authorization_code flow and update the `refresh_token` in the secrets store. The Azure app must be configured as a PUBLIC client (no `client_secret`).

### Email (Microsoft Graph API)

**Graph API 401**

Cause: Token refresh failed, or the Azure app permissions are misconfigured.
Fix: Verify the Azure app is registered as a PUBLIC client. Re-authenticate to obtain a new refresh token and update it in secrets.

---

## 6. Pipeline Issues

### send_payload silently fails (no message sent)

**Symptom:** The pipeline runs through ingress, app, render, and encode stages without errors, but no outbound message appears in the target channel.

**Cause:** `SendPayloadInV1.provider_type` uses the wrong delimiter. The pipeline expects dot-separated types (e.g., `messaging.telegram`) but may receive hyphen-separated types (e.g., `messaging-telegram`).

**Fix:** Ensure `provider_type` uses dots as delimiters: `messaging.webchat`, `messaging.telegram`, `messaging.webex`, etc.

### Encode returns "invalid encode input"

**Symptom:** The encode stage fails with deserialization errors.

**Cause:** The WASM component was compiled against an older `greentic-types` version and the input payload format has changed.

**Fix:** Rebuild the provider WASM and update the gtpack. See section 3.

### App flow returns empty text

**Symptom:** The echo bot reply message has no text content.

**Cause:** The runner wraps the response text under `payload.text` rather than at the top level of the output. If the next pipeline stage reads from the wrong path, the text appears empty.

**Fix:** This is handled by fallback extraction logic in the operator. If you encounter this with a custom flow, check that your flow's output template matches the runner's output structure.

---

## 7. gtpack Issues

### `zip -j` creates corrupt archive entries

**Symptom:** `zipinfo` shows paths with `^J` (newline) characters embedded in filenames.

**Cause:** Using `zip -j` (junk paths) with certain inputs produces malformed entries inside the gtpack archive.

**Fix:** Use a temporary directory to construct the correct archive structure:

```bash
tmpdir=$(mktemp -d)
mkdir -p "$tmpdir/components/messaging-provider-telegram"
cp new-provider.wasm "$tmpdir/components/messaging-provider-telegram/component.wasm"
(cd "$tmpdir" && zip -u /path/to/messaging-telegram.gtpack "components/messaging-provider-telegram/component.wasm")
rm -rf "$tmpdir"
```

Verify the result:

```bash
zipinfo /path/to/messaging-telegram.gtpack | grep components/
# Should show clean paths like: components/messaging-provider-telegram/component.wasm
```

### `greentic-pack build` fails with OCI error

**Symptom:** Pack build fails trying to pull component artifacts from `ghcr.io/greentic-ai`.

**Cause:** Not authenticated to the GitHub Container Registry, or the component images are not publicly accessible.

**Fix:** Use the `--offline` flag to skip OCI pulls and rely on local component paths:

```bash
greentic-pack build --offline
```

Some components (like `component-adaptive-card`) may require OCI access if they are not available locally.

---

## Quick Reference

| Problem | First thing to check |
|---------|---------------------|
| "Address already in use" | `fuser -k 8080/tcp` |
| "failed to open dev store" | `export GREENTIC_ENV=dev` |
| "secret not found" | Tenant mismatch (`default` vs `demo`) |
| "MAC check failed" | DEK cache bug -- re-seed all secrets in one batch |
| Webhooks not arriving | Get fresh tunnel URL from `demo-bundle/logs/cloudflared.log` |
| "invalid encode input" | Rebuild WASM components (stale cache) |
| No message sent | Check `provider_type` uses dots, not hyphens |
| gtpack corrupt | Use temp directory method for `zip -u`, not `zip -j` |
