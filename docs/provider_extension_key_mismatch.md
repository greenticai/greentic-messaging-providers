# Provider extension key mismatch

- Decoded `dist/packs/messaging-slack.gtpack` `manifest.cbor` and found the only `extensions` entry is keyed `greentic.ext.provider` with inline provider metadata (capabilities, schemas, runtime world) — the canonical greentic-types key should be `greentic.provider-extension.v1`.
- Pack sources embed this legacy key directly in both YAML and JSON: `packs/messaging-slack/pack.yaml:20-37` and `packs/messaging-slack/pack.manifest.json:16-44` (same pattern across all packs).
- `tools/generate_pack_metadata.py:147-157` simply copies the `extensions` map from `pack.yaml` into `pack.manifest.json` without translating or validating against greentic-types; this Python script is what feeds `packc build`, so the packed `manifest.cbor` retains the legacy key.
- Tests also assert the legacy key, e.g., `crates/provider-tests/tests/provider_core_slack.rs:180-206`, so the current expected shape is locked to `greentic.ext.provider`.
- Manifest assembly does not flow through greentic-types constants at all; greentic-types `0.4.28` is only pulled into the component crates (see `cargo tree -i greentic-types`), while pack metadata is authored by hand plus the Python helper, which predates or ignores the canonical `greentic.provider-extension.v1`.
