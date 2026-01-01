# PR-04: CI build for providers repo

## Goal
Add GitHub Actions workflow that builds and tests the workspace and produces component artifacts.

## Tasks
1) Add .github/workflows/build.yml:
- runs on push + PR
- steps:
  - checkout
  - install Rust toolchain (match org standard)
  - add wasm32-wasip2 target (or required component build tooling)
  - cargo fmt --check
  - cargo test --workspace
  - ./tools/build_components.sh
- upload artifacts from target/components/*.wasm (as workflow artifact)

2) Document in README:
- how to build locally
- where artifacts appear

## Acceptance
- CI passes on PR
- artifacts uploaded
