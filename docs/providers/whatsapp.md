# WhatsApp Provider

## What It Does

WhatsApp connects Greentic to WhatsApp Business through the Meta WhatsApp Cloud API.

## Features

- Sends WhatsApp text messages through the Cloud API.
- Supports media-oriented envelope fields where implemented by provider code.
- Handles WhatsApp webhook verification.
- Handles Cloud API webhook message payloads.
- Converts rich content to WhatsApp-safe text or supported media fields.
- Supports nightly e2e metadata for a dedicated test phone number.

## Setup Inputs

Common setup values include:

- Public base URL for webhooks.
- WhatsApp phone number ID.
- Default destination phone number for tests or simple bundles.
- Cloud API token secret.
- Optional webhook verify token.

## Secrets

| Secret | Required | Purpose |
| --- | --- | --- |
| `WHATSAPP_TOKEN` | Yes | WhatsApp Cloud API access token. |
| `WHATSAPP_VERIFY_TOKEN` | Optional, recommended for ingress | Token used during webhook verification. |
| `WHATSAPP_APP_SECRET` | Optional, needed for inbound files | Meta app secret. The host verifies each webhook's `X-Hub-Signature-256` with it; without it text still flows but files are not read. |

Nightly e2e uses `E2E_WHATSAPP_TOKEN`, `E2E_WHATSAPP_PHONE_NUMBER_ID`, and `E2E_WHATSAPP_TO`.

## Message Features

- Outbound: WhatsApp Cloud API messages.
- Inbound: webhook verification and Cloud API event payloads.
- Replies: supported when context metadata is available.
- Adaptive Cards: converted to WhatsApp-safe fallback content.
- External read-back: send API response ID validation is the baseline.

## Owned Files

- `components/whatsapp/`
- `components/messaging-ingress-whatsapp/`
- `components/messaging-provider-whatsapp/`
- `packs/messaging-whatsapp/`
- `crates/provider-tests/tests/provider_core_whatsapp.rs`
- `e2e/providers/whatsapp/`

## Focused Checks

```bash
cargo test -p messaging-provider-whatsapp
cargo test -p messaging-ingress-whatsapp
cargo test -p provider-tests provider_core_whatsapp
PACK_FILTER=messaging-whatsapp ./ci/steps/11_build_packs.sh
```

## Agent Notes

Do not use personal or production phone numbers in committed examples. Use dedicated sandbox or test numbers for live checks.

