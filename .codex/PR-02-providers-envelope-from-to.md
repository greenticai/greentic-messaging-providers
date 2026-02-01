# PR: greentic-messaging-providers — Switch `send` to accept `ChannelMessageEnvelope` with `to[]` + `from`

## Summary
Update provider components to:
- accept `ChannelMessageEnvelope` as the `send` input (JSON)
- resolve destinations from `envelope.to` (first) then provider config default, else error
- stop parsing ad-hoc `to` objects in send JSON
- ingress `ingest_http` should populate `from` (actor) where possible

Start with Webex provider as the reference implementation, then apply the same pattern to other providers.

## Global conventions

### Destination resolution algorithm (egress)
For `send`:
1. If `envelope.to` is non-empty: use the **first** destination (broadcast can be a future extension).
2. Else if provider config has default destination (e.g. `default_room_id`): synthesize `Destination { kind: Some("room"), id: default }`.
3. Else: return error `"destination required"`.

### Kind mapping
Provider chooses its accepted destination kinds:
- Webex: `room` or `user`
- Telegram: `chat`
- Slack: `channel` (and optionally `user` if provider resolves DM channel)
- Teams: `channel` where `id` = `teamId:channelId` OR `chat` where `id` = `chatId`
- WhatsApp: `phone`
- Email: `email`

## Webex provider changes (example)

### 1) Replace `handle_send(input_json: &[u8])` parsing
Current code parses `serde_json::Value` and expects:
- `to.kind` / `to.id`
- `text|markdown`
- config embedded in request

**New behavior:**
- deserialize `ChannelMessageEnvelope` from input JSON (using greentic-types)
- read text from `envelope.text` (required; optional markdown can remain metadata-based if needed)
- resolve destination from `envelope.to` or config default

Suggested skeleton:

```rust
use greentic_types::{ChannelMessageEnvelope, Destination};

fn handle_send(input_json: &[u8]) -> Vec<u8> {
    let envelope: ChannelMessageEnvelope = match serde_json::from_slice(input_json) {
        Ok(v) => v,
        Err(err) => return json_bytes(&json!({"ok": false, "error": format!("invalid envelope: {err}")})),
    };

    if !envelope.attachments.is_empty() {
        return json_bytes(&json!({"ok": false, "error": "attachments not supported"}));
    }

    // Load config: prefer envelope.metadata/config field if you keep it; otherwise provider-level config retrieval
    // If you still support request-embedded config during transition, keep `load_config(&Value)` but build Value from envelope.metadata or a top-level config object.
    // Recommended: keep existing `load_config` logic for now but call it on a constructed Value that includes {"config": ...} if present.

    let text = envelope.text.clone().unwrap_or_default();
    if text.trim().is_empty() {
        return json_bytes(&json!({"ok": false, "error": "text required"}));
    }

    let (kind, id) = match envelope.to.first() {
        Some(dest) => (dest.kind.clone().unwrap_or_else(|| "room".to_string()), dest.id.clone()),
        None => {
            // fallback to cfg.default_room_id
        }
    };

    // Webex mapping:
    // - kind == "room" => roomId = id
    // - kind == "user" => personId = id
    // else error

    // ... send HTTP as before ...
}
```

### 2) Ensure `ingest_http` populates `from`
Where you build `ChannelMessageEnvelope`, set:

- `from: Some(Actor { id: person_id_or_sender_id, kind: Some("user") })` when available
- `to: vec![]` for inbound events (destinations not needed)

### 3) Ensure `build_envelope()` signature changes
Current signature has `person_id` which is used for `user_id`.
Update it to accept `from: Option<Actor>` or `from_id`.

Example:

```rust
fn build_envelope(...) -> ChannelMessageEnvelope {
    ChannelMessageEnvelope {
        // ...
        from: person_id.map(|id| Actor { id, kind: Some("user".into()) }),
        to: Vec::new(),
        // ...
    }
}
```

## Repeat for other providers
For each provider component:
- Update `send` input parsing to `ChannelMessageEnvelope`
- Use `envelope.to[0]` + optional kind + composite id parsing if required
- Ensure inbound mapping sets `from` not `user_id`
- Ensure all envelope constructors add `to: vec![]` explicitly (if not defaulted)

## Tests
Update provider tests to use the new envelope.

Suggested minimal regression tests per provider:
- `send` returns destination-required error when `to` empty and no config default
- `send` succeeds (or at least builds correct HTTP request in mock) when `to` present

Run:
```bash
cargo fmt
cargo test -p provider-tests -- --nocapture
```

## Acceptance criteria
- All providers compile with updated greentic-types
- Webex `send` uses `ChannelMessageEnvelope.to` (no JSON `to` parsing)
- Ingress events populate `from` not `user_id`
- Operator demo send can drive at least one provider via `send` with a destination
