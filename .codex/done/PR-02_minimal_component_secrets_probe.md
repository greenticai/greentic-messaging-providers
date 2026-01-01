# PR-02: Add minimal provider component (secrets probe) to prove WIT + build

## Goal
Add the first runnable WASM component that imports greentic:secrets-store@1.0.0 and calls get("TEST_API_KEY").
This is not a real provider yet; it proves the toolchain.

## Tasks
1) Create component crate:
- components/secrets-probe/
  - Cargo.toml
  - src/lib.rs
  - wit/ layout (structurally valid WIT packages)

2) WIT layout MUST be valid:
wit/
  secrets-probe/
    world.wit
  deps/
    greentic/
      secrets-store/
        1.0.0/
          package.wit

- world.wit defines package probe:secrets-probe@0.0.1 and a world that imports secrets-store and exports run() -> string
- package.wit defines greentic:secrets-store@1.0.0 with interface secrets-store { get(key) -> result<option<list<u8>>, secrets-error> } (minimal)

3) Rust code:
- use wit_bindgen::generate! pointing at the directory wit/secrets-probe, world "secrets-probe"
- implement exported run() that calls secrets_store.get("TEST_API_KEY")
  - on Some(_) return JSON string {"ok":true,"key_present":true}
  - on None or not-found return JSON string {"ok":false,"key_present":false}

4) Component manifest:
- components/secrets-probe/component.manifest.json
  - include structured secret_requirements with key TEST_API_KEY (tenant scope)
  - DO NOT read env vars

5) Build target:
- add a simple `tools/build_components.sh` that builds this component to:
  - target/components/secrets-probe.wasm
Use the repo’s standard wasm build approach (cargo component / wasm32-wasip2). Pick the simplest that works in your environment; make it deterministic.

6) Add a compile-only/unit test if appropriate (optional), but ensure workspace tests remain green.

## Acceptance
- `./tools/build_components.sh` produces target/components/secrets-probe.wasm
- `cargo test --workspace` passes
- no env-based secret reads exist

## Notes
- This is the reference component for later CI and for greentic-messaging pack builds.
- Keep it minimal and deterministic.
