# Messaging CardKit

`messaging-cardkit` packages the shared MessageCard rendering, tier downgrade, and platform renderer registry extracted from `gsm-core` so operator/runner runtimes can integrate without pulling in GSM gateway/egress or NATS dependencies. The crate also introduces a `ProfileSource` abstraction so pack metadata (or fixed mappings) can drive renderer selection without coupling to pack loaders.

## What it contains

* `CardKit` – lightweight entry point with `render(provider_type, card_json)` and `render_with_spec(provider_type, spec)` helpers that mirror the existing Dev Viewer render pipeline.
* Profile metadata helpers (`ProfileSource`, `StaticProfiles`, `StaticProfilesBuilder`, `PackProfiles`) that convert provider types or parsed `ProviderDecl` metadata into renderer tiers/capability profiles.
* Stable output types (`RenderResponse`, `PlatformPreview`) that capture the rendered payload, intent, warnings, tier metadata, downgrade flag, and provider capability profile.
* Re-exported core components: `MessageCardEngine`, `MessageCard`, `RenderSpec`, renderer variants (`SlackRenderer`, `TeamsRenderer`, `WebChatRenderer`, `WebexRenderer`, `TelegramRenderer`, `WhatsAppRenderer`), and capability helpers.

## API contract

```rust
use messaging_cardkit::{
    CardKit, CapabilityProfile, StaticProfiles, StaticProfilesBuilder, Tier,
};
use serde_json::json;
use std::sync::Arc;

let profiles = Arc::new(
    StaticProfiles::builder()
        .default_tier(Tier::Basic)
        .for_provider("slack", Tier::Premium)
        .build(),
);
let kit = CardKit::new(profiles);
let response = kit.render(
    "slack",
    &json!({
        "kind": "standard",
        "title": "Hello",
        "text": "Message",
    }),
)?;

assert!(response.payload.get("blocks").is_some());
assert_eq!(response.capability, Some(CapabilityProfile::for_tier(Tier::Premium)));
```

`RenderResponse` mirrors the shape emitted by `tools/dev-viewer` so existing tooling can adopt it without parallel structs. Platform previews provide telemetry data (`tier`, `target_tier`, `downgraded`, `warnings`, `used_modal`, etc.) for downstream reporting, and `RenderResponse::capability` exposes the provider's `CapabilityProfile`.

## Profile sources

`CardKit` delegates tier/capability lookup to any type that implements `ProfileSource`, so an operator can provide a shared registry instead of rebuilding pack discovery inside this crate. Use `StaticProfiles`/`StaticProfilesBuilder` for fixed mappings (great for demos) or build a `PackProfiles` from parsed `greentic_types::provider::ProviderDecl` metadata to let each pack dictate renderer tiers.

```rust
use greentic_types::pack_manifest::PackManifest;
use messaging_cardkit::{CardKit, PackProfiles};
use std::sync::Arc;

let manifest: PackManifest = ...;
let provider_decls = manifest
    .provider_extension_inline()
    .map(|inline| inline.providers.clone())
    .unwrap_or_default();
let profiles = Arc::new(PackProfiles::new(provider_decls));
let kit = CardKit::new(profiles);
```

`PackProfiles` works purely on already-parsed `ProviderDecl` values, so pack loaders (like `tools/dev-viewer` or operator runtimes) stay responsible for manifest/gtpack handling while this crate focuses on rendering and downgrade logic. Implementers may also override `ProfileSource::button_limit` to expose renderer-specific limits that inform downstream UI.

## Capabilities & tiers

`CardKit` relies on the same `MessageCardEngine` downgrade logic as before: renderers expose a `target_tier`, and the engine rewrites the IR via `PolicyDowngradeEngine` when the card tier is higher. Consumers should continue to use `CapabilityProfile::for_tier` when implementing renderer-aware guards; `ProfileSource` simply lets you supply provider-specific tiers/capabilities instead of hard-coding them into the renderer selection code path.

## CLI & render server

The optional `messaging-cardkit-bin` workspace crate exposes the same `ProfileSource` surface plus a mini render demo CLI and HTTP server so downstream tooling can run CardKit without GSM gateway/egress features. The CLI command prints the full `RenderResponse` JSON (intent, payload, preview metadata, warnings, capability) so operators can inspect renderer behavior for any fixture.

```bash
cargo run -p messaging-cardkit-bin -- render \
  --provider slack \
  --fixture crates/messaging-cardkit/tests/fixtures/cards/basic.json
```

The `serve` command starts an HTTP endpoint (default `127.0.0.1:7878`, `tests/fixtures/cards`):

```bash
cargo run -p messaging-cardkit-bin -- serve --port 7879 --fixtures-dir ./tests/fixtures/cards
```

The server exposes:

* `GET /providers` – returns configured provider overrides plus the default tier.
* `POST /render` – accepts `{"provider": "...", "fixture": "basic.json"}` or an inline `card` payload and returns the same `RenderResponse` structure that `CardKit` emits.

Pass `--default-tier` and `--provider-tier <provider=Tier>` to mirror operator tier policies while only shipping the slim `messaging-cardkit` crate.

## Fixture-based golden tests

`CardKit` now bundles its Adaptive Card fixtures (`tests/fixtures/cards/*.json`) and renderer snapshots (`tests/fixtures/renderers/<provider>.json`) so the crate can verify every platform’s deterministic output without touching external services. The `renderers_match_golden_payloads` test converts the `basic` card via `gsm_core::messaging_card::adaptive::normalizer`, renders each provider through `CardKit`, and asserts the payload, warnings, and `downgraded` flag match the stored fixtures. Update the renderer snapshots whenever the renderers intentionally evolve and keep the fixture JSON checked in so downstream operators can reproduce the behavior offline.

## Running tests

This crate contains fixture coverage for each renderer. Run `cargo test -p messaging-cardkit` from the workspace root once your network/cache can fetch `anyhow` and other registry crates. In offline environments, the tests currently fail because Cargo cannot resolve `index.crates.io`.

## Next steps

- Keep the `tests/fixtures/renderers/*` snapshots aligned with upstream renderer tweaks so the golden test stays deterministic.
- Extend your runner/operator to feed real `ProviderDecl` data into `PackProfiles` (or the demo CLI/server above) when additional renderer tiers are introduced.
