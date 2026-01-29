# messaging-cardkit-bin

`messaging-cardkit-bin` is the companion CLI/HTTP server for the `messaging-cardkit` renderer crate. It reuses the golden fixtures that ship with the library so operators and tooling can inspect how each provider is rendered without standing up the full GSM gateway stack.

## CLI usage
```
cargo run -p messaging-cardkit-bin -- <SUBCOMMAND> [OPTIONS]
```
### `render`
- `--provider <provider>` – provider type (e.g., `slack`, `teams`, `webchat`).
- `--fixture <path>` – JSON fixture path relative to the local fixture directory (default `tests/fixtures/cards`).
- `--default-tier <tier>` – default tier (`basic`, `advanced`, `premium`).
- `--provider-tier <provider=tier>` – override tier for a specific provider (repeatable).

The command prints the full `RenderResponse` JSON (intent, payload, preview metadata, warnings, capability profile) so you can inspect downgrade behavior for any fixture.

### `serve`
- `--host` (default `127.0.0.1`), `--port` (default `7878`).
- `--fixtures-dir` (default `tests/fixtures/cards`).

Starts a minimal HTTP service with two endpoints:
- `POST /render` accepts `{"provider":"slack","fixture":"basic.json"}` or inline `card` payload and proxies the renderer.
- `GET /providers` returns the configured provider overrides plus the default tier.

## Fixtures and assets
Default fixtures live in `crates/messaging-cardkit-bin/tests/fixtures/cards` (adaptive cards) and the renderer snapshots under `tests/fixtures/renderers`. The server and CLI rely on `--fixtures-dir` for determinism, so CI/dev shells should keep those files checked in.

## Testing
- `cargo test -p messaging-cardkit-bin` runs the smoke tests bundled with the binary.
- The library targets `messaging-cardkit` golden tests (`cards/*.json`, `renderers/*.json`), and those fixtures are reused by the server examples.

## Environment and configuration
No credentials or environment variables are required; everything is driven through CLI arguments and the in-tree fixtures. The crate depends on `messaging-cardkit`, which itself builds on `gsm-core` and `greentic-types` from the workspace.
