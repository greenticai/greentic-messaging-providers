# CardKit CLI: Preview Provider-Specific Card Output

CardKit is a standalone tool for rendering Adaptive Cards into provider-specific formats (Telegram, Slack, Teams, etc.) with tier-based downsampling.

## Overview

The Adaptive Card spec (v1.3+) is rich, but not all providers support the full spec. CardKit takes an Adaptive Card JSON and produces the provider-specific payload, showing what gets downsampled, blocked, or converted.

```
Adaptive Card JSON
    |
    v
CardKit engine (tier + capability profile)
    |
    +-- Telegram: sendMessage + inline_keyboard
    +-- Slack: blocks JSON
    +-- Teams: full AC attachment
    +-- WebChat: full AC attachment
    +-- Webex: AC attachment
    +-- WhatsApp: text + buttons
```

## Location

All CardKit crates live in `greentic-messaging-providers/crates/`:

| Crate | Purpose |
|-------|---------|
| `messaging-cardkit` | Core library (rendering engine, profiles, tiers) |
| `messaging-cardkit-bin` | CLI binary + HTTP server |
| `greentic-messaging-cardkit` | Thin CLI wrapper (workspace member) |

## Build

```bash
cd greentic-messaging-providers

# Build the CLI
cargo build --release --package messaging-cardkit-bin

# Binary at:
# target/release/messaging-cardkit-bin

# Or via the wrapper:
cargo build --release --package greentic-messaging-cardkit
# Binary at: target/release/greentic-messaging-cardkit
```

## CLI Usage

### `render` -- Render a single card

Render an Adaptive Card JSON through a specific provider profile.

The `--provider` flag uses short names: `telegram`, `slack`, `teams`, `webchat`, `webex`, `whatsapp` (not `messaging-telegram`).

The `--fixture` path is relative to where you run the command:

```bash
cd greentic-messaging-providers

# Run from the repo root — fixture paths are relative
target/release/messaging-cardkit-bin render \
  --provider telegram \
  --fixture crates/messaging-cardkit/tests/fixtures/cards/basic.json
```

Output (JSON):

```json
{
  "intent": "send",
  "payload": { ... },
  "preview": {
    "payload": {
      "method": "sendMessage",
      "parse_mode": "HTML",
      "text": "Hello from fixtures",
      "reply_markup": {
        "inline_keyboard": [
          [{"text": "Docs", "url": "https://example.com/docs"}]
        ]
      }
    },
    "tier": "Basic",
    "target_tier": "Basic",
    "downgraded": false,
    "used_modal": false,
    "limit_exceeded": false,
    "sanitized_count": 0,
    "url_blocked_count": 0,
    "warnings": []
  },
  "warnings": [],
  "capability": {
    "allow_images": true,
    "allow_factset": false,
    "allow_inputs": false,
    "allow_postbacks": true
  }
}
```

### Compare across providers

```bash
cd greentic-messaging-providers

for provider in telegram slack teams webchat webex whatsapp; do
  echo "=== $provider ==="
  target/release/messaging-cardkit-bin render \
    --provider "$provider" \
    --fixture crates/messaging-cardkit/tests/fixtures/cards/basic.json \
    | jq '.preview.payload'
done
```

### Override tier

Tiers control capability levels:

| Tier | Description |
|------|-------------|
| `basic` | Text + buttons only (Telegram, WhatsApp) |
| `advanced` | Rich formatting, images, some inputs (Slack, Webex) |
| `premium` | Full Adaptive Card support (Teams, WebChat) |

```bash
# Force all providers to basic tier
messaging-cardkit-bin --default-tier basic render \
  --provider teams \
  --fixture tests/fixtures/cards/premium.json

# Override specific providers
messaging-cardkit-bin \
  --default-tier basic \
  --provider-tier teams=premium \
  --provider-tier webchat=premium \
  render --provider teams --fixture tests/fixtures/cards/inputs.json
```

### `serve` -- HTTP render server

Start an HTTP server for interactive testing:

```bash
cd greentic-messaging-providers

target/release/messaging-cardkit-bin serve \
  --host 127.0.0.1 \
  --port 7878 \
  --fixtures-dir crates/messaging-cardkit/tests/fixtures/cards
```

Endpoints:

| Method | Path | Body | Purpose |
|--------|------|------|---------|
| `POST` | `/render` | `{"provider": "telegram", "fixture": "basic.json"}` | Render from fixture file |
| `POST` | `/render` | `{"provider": "slack", "card": {...}}` | Render inline card |
| `GET` | `/providers` | - | List configured provider tiers |

```bash
# Render from fixture
curl -s -X POST http://localhost:7878/render \
  -H "Content-Type: application/json" \
  -d '{"provider": "telegram", "fixture": "basic.json"}' | jq .

# Render inline card
curl -s -X POST http://localhost:7878/render \
  -H "Content-Type: application/json" \
  -d '{
    "provider": "slack",
    "card": {
      "type": "AdaptiveCard",
      "version": "1.3",
      "body": [
        {"type": "TextBlock", "text": "Test card"},
        {"type": "FactSet", "facts": [
          {"title": "Key", "value": "Value"}
        ]}
      ]
    }
  }' | jq '.preview.payload'

# List providers
curl -s http://localhost:7878/providers | jq .
```

## Test Fixtures

Fixtures are in `greentic-messaging-providers/crates/messaging-cardkit/tests/fixtures/`:

### Card fixtures (`cards/`)

| File | Content |
|------|---------|
| `basic.json` | TextBlock + Action.OpenUrl |
| `columns.json` | ColumnSet layout |
| `execute.json` | Action.Execute (converted to Submit for non-AC providers) |
| `facts.json` | FactSet |
| `inputs.json` | Input.Text + Input.ChoiceSet |
| `inputs_showcard.json` | Inputs inside Action.ShowCard |
| `showcard.json` | Action.ShowCard |
| `premium.json` | Full-spec card with all element types |
| `generated_markdown.json` | Markdown-heavy content |

### Renderer configs (`renderers/`)

Each JSON defines the expected output format for a provider:

| File | Provider |
|------|----------|
| `telegram.json` | Telegram Bot API sendMessage format |
| `slack.json` | Slack blocks format |
| `teams.json` | Teams/Bot Framework Adaptive Card attachment |
| `webchat.json` | WebChat Direct Line Adaptive Card attachment |
| `webex.json` | Webex Adaptive Card attachment |
| `whatsapp.json` | WhatsApp Cloud API text/buttons format |

## Understanding the Output

Key fields in the render response:

```json
{
  "intent": "send",           // or "noop" if card is empty
  "payload": {},              // full provider payload (what gets sent to the API)
  "preview": {
    "payload": {},            // platform-specific preview
    "tier": "Basic",          // provider's actual tier
    "target_tier": "Basic",   // requested tier
    "downgraded": false,      // true if card was simplified
    "used_modal": false,      // true if Action.ShowCard was used
    "limit_exceeded": false,  // true if content was truncated
    "sanitized_count": 0,     // number of elements removed
    "url_blocked_count": 0,   // number of URLs blocked
    "warnings": []            // any rendering warnings
  },
  "capability": {
    "allow_images": true,
    "allow_factset": false,   // true only for AC-native providers
    "allow_inputs": false,    // true only for premium tier
    "allow_postbacks": true
  }
}
```

When `downgraded: true`, the card was too rich for the target tier. Check `sanitized_count` and `warnings` to see what was removed.

## Running the Smoke Tests

```bash
cd greentic-messaging-providers
cargo test --package messaging-cardkit
```

This runs the smoke tests in `crates/messaging-cardkit/tests/smoke.rs` which verify rendering across all fixtures and providers.
