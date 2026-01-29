# Changelog

All notable changes to `messaging-cardkit` are documented in this file. The crate follows semantic versioning and guarantees that `RenderResponse`, `PlatformPreview`, and the `ProfileSource` contracts remain stable between patch releases. Update this changelog before publishing any new version and mention the relevant PRs/features.

## 0.1.0 (January 27, 2026)

### Added

- Introduced the `ProfileSource` trait plus `StaticProfiles`/`PackProfiles` so operators can map provider metadata to renderer tiers without pulling in pack loaders or the GSM gateway stack (PR-MSG-02).
- Bundled deterministic Adaptive Card fixtures and per-platform renderer snapshots, and added `renderers_match_golden_payloads` so every provider’s payload/warnings/downgrade behavior is reproducible inside this crate (PR-MSG-03).
- Added the `messaging-cardkit-bin` CLI & HTTP server demo that renders fixtures via the same `RenderResponse` contract and exposes `/render` and `/providers` endpoints, enabling render previews without extra GSM features (PR-MSG-04).
- Documented the API expectations, fixture usage, and CLI/server workflow so downstream operator teams can integrate `messaging-cardkit` independently (PR-MSG-05).

### Notes

- When evolving renderer behavior, update both the golden fixtures under `tests/fixtures/renderers` and the changelog above so consumers know to refresh their snapshots.
