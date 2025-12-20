# PR-01: Scaffold greentic-messaging-providers workspace

## Goal
Create a new Rust workspace that can host multiple provider WASM components and shared crates.

## Tasks
1) Create workspace layout:
- Cargo workspace root
- crates/messaging-core (shared message types, helpers)
- crates/provider-common (shared provider utilities)
- components/ (empty for now)
- tools/ (empty for now)
- .github/workflows/ (empty for now)

2) Add basic root files:
- README.md (describe purpose: this repo builds provider components; packs are built elsewhere)
- MIGRATION_STATUS.md (initial entry)
- LICENSE (MIT)

3) Root Cargo.toml
- [workspace] members include crates/* and components/* (even if empty now)
- Set workspace edition to match your org standard (likely 2024 if you’re already there)

4) Add minimal code so `cargo test --workspace` works:
- messaging-core: define a basic Message struct + serde derives
- provider-common: define a ProviderError enum + thiserror + serde
- Keep dependencies minimal.

## Acceptance
- `cargo fmt` passes
- `cargo test --workspace` passes
- repo has the expected structure and docs

## Notes
- Do not add provider components yet; that’s PR-02.
- Make best-effort choices without asking questions; do not leave TODOs that block build.
