use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use greentic_types::provider::PROVIDER_EXTENSION_ID;
use serde_json::Value;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

fn version_from_yaml(path: &PathBuf) -> Result<String> {
    let contents = fs::read_to_string(path).context("reading pack.yaml")?;
    for line in contents.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("version:") {
            return Ok(rest.trim().to_string());
        }
    }
    Err(anyhow::anyhow!("version not found in {}", path.display()))
}

#[test]
fn packs_have_consistent_manifests_and_artifacts() -> Result<()> {
    let root = workspace_root();
    let packs_dir = root.join("packs");
    for entry in fs::read_dir(&packs_dir).context("reading packs dir")? {
        let entry = entry?;
        let pack_dir = entry.path();
        if !pack_dir.is_dir() {
            continue;
        }

        let yaml_path = pack_dir.join("pack.yaml");
        let manifest_path = pack_dir.join("pack.manifest.json");
        assert!(
            yaml_path.exists(),
            "missing pack.yaml for {}",
            pack_dir.display()
        );
        assert!(
            manifest_path.exists(),
            "missing pack.manifest.json for {}",
            pack_dir.display()
        );

        let yaml_version = version_from_yaml(&yaml_path)
            .with_context(|| format!("getting version from {}", yaml_path.display()))?;
        let manifest: Value = serde_json::from_slice(&fs::read(&manifest_path)?)
            .with_context(|| format!("parsing {}", manifest_path.display()))?;
        let manifest_version = manifest
            .get("version")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("manifest missing version"))?;
        assert_eq!(
            manifest_version,
            yaml_version,
            "pack version mismatch for {}",
            pack_dir.display()
        );

        // Components in manifest must have wasm artifacts staged in the pack.
        if let Some(comps) = manifest.get("components").and_then(Value::as_array) {
            for comp in comps {
                let name = comp
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("component entry must be string"))?;
                let wasm = pack_dir.join("components").join(format!("{name}.wasm"));
                assert!(
                    wasm.exists(),
                    "missing component artifact {} for {}",
                    wasm.display(),
                    pack_dir.display()
                );
                assert!(
                    wasm.metadata()?.len() > 0,
                    "component artifact {} is empty",
                    wasm.display()
                );
            }
        }

        // Provider extension config schemas must exist in pack and workspace.
        let provider_ext = manifest
            .get("extensions")
            .and_then(|ext| ext.get(PROVIDER_EXTENSION_ID))
            .unwrap_or_else(|| {
                panic!(
                    "pack {} missing provider extension {}",
                    pack_dir.display(),
                    PROVIDER_EXTENSION_ID
                )
            });

        let kind = provider_ext
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert_eq!(
            kind,
            PROVIDER_EXTENSION_ID,
            "pack {} provider extension kind mismatch",
            pack_dir.display()
        );

        if let Some(providers) = provider_ext
            .get("inline")
            .and_then(|inline| inline.get("providers"))
            .and_then(Value::as_array)
        {
            for provider in providers {
                if let Some(schema) = provider.get("config_schema_ref").and_then(Value::as_str) {
                    let pack_schema = pack_dir.join(schema);
                    assert!(
                        pack_schema.exists(),
                        "pack schema missing: {}",
                        pack_schema.display()
                    );
                    let root_schema = root.join(schema);
                    assert!(
                        root_schema.exists(),
                        "workspace schema missing: {}",
                        root_schema.display()
                    );
                }
                if let Some(component_ref) = provider
                    .get("runtime")
                    .and_then(|rt| rt.get("component_ref"))
                    .and_then(Value::as_str)
                {
                    let wasm = pack_dir
                        .join("components")
                        .join(format!("{component_ref}.wasm"));
                    assert!(
                        wasm.exists(),
                        "runtime component {} missing for {}",
                        component_ref,
                        pack_dir.display()
                    );
                }
            }
        }

        // Config schema in manifest must exist in pack and workspace.
        if let Some(cfg) = manifest
            .get("config_schema")
            .and_then(|v| v.get("provider_config"))
            .and_then(|v| v.get("path"))
            .and_then(Value::as_str)
        {
            let pack_schema = pack_dir.join(cfg);
            assert!(
                pack_schema.exists(),
                "pack config schema missing: {}",
                pack_schema.display()
            );
            let root_schema = root.join(cfg);
            assert!(
                root_schema.exists(),
                "workspace config schema missing: {}",
                root_schema.display()
            );
        }
    }

    Ok(())
}

#[test]
fn bundle_pack_has_unique_providers_and_schema_paths() -> Result<()> {
    let root = workspace_root();
    let pack_dir = root.join("packs").join("messaging-provider-bundle");
    let manifest_path = pack_dir.join("pack.manifest.json");
    let manifest: Value = serde_json::from_slice(&fs::read(&manifest_path)?)?;

    let provider_ext = manifest
        .get("extensions")
        .and_then(|ext| ext.get(PROVIDER_EXTENSION_ID))
        .and_then(|ext| ext.get("inline"))
        .and_then(|inline| inline.get("providers"))
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("bundle pack missing providers extension"))?;

    let mut seen = std::collections::BTreeSet::new();
    let mut providers = Vec::new();
    for provider in provider_ext {
        let provider_type = provider
            .get("provider_type")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("bundle provider missing provider_type"))?;
        assert!(
            seen.insert(provider_type.to_string()),
            "duplicate provider id in bundle pack: {}",
            provider_type
        );
        providers.push(provider_type.to_string());

        if let Some(schema) = provider.get("config_schema_ref").and_then(Value::as_str) {
            assert!(
                schema.starts_with("schemas/messaging/"),
                "bundle provider schema ref must be canonical, got: {}",
                schema
            );
        }
    }

    let expected = [
        "messaging.slack.api",
        "messaging.teams.graph",
        "messaging.telegram.bot",
        "messaging.webchat",
        "messaging.webex.bot",
        "messaging.whatsapp.cloud",
    ];
    let expected_set = expected
        .iter()
        .map(|id| id.to_string())
        .collect::<std::collections::BTreeSet<_>>();
    let found_set = providers
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(expected_set, found_set, "bundle pack providers mismatch");

    Ok(())
}
