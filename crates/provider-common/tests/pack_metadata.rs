use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Result, anyhow};
use serde_json::{Map, Value};
use tempfile::tempdir;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

fn copy_dir(src: &Path, dest: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let target = dest.join(entry.file_name());
        if path.is_dir() {
            copy_dir(&path, &target)?;
        } else {
            fs::copy(&path, &target)?;
        }
    }
    Ok(())
}

fn build_gtpack(src_dir: &Path, output: &Path) -> Result<()> {
    let file = fs::File::create(output)?;
    let mut zip = zip::ZipWriter::new(file);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

    let mut stack = vec![src_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let rel = path.strip_prefix(src_dir).expect("relative path");
            let mut contents = Vec::new();
            fs::File::open(&path)?.read_to_end(&mut contents)?;
            zip.start_file(rel.to_string_lossy(), options)?;
            zip.write_all(&contents)?;
        }
    }

    zip.finish()?;
    Ok(())
}

fn read_from_gtpack(gtpack: &Path, file: &str) -> Result<Vec<u8>> {
    let archive = fs::File::open(gtpack)?;
    let mut zip = zip::ZipArchive::new(archive)?;
    let mut file = zip.by_name(file)?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    Ok(buf)
}

fn list_flow_json_entries(gtpack: &Path) -> Result<Vec<String>> {
    let archive = fs::File::open(gtpack)?;
    let zip = zip::ZipArchive::new(archive)?;
    Ok(zip
        .file_names()
        .filter(|name| name.starts_with("flows/") && name.ends_with("/flow.json"))
        .map(|name| name.to_string())
        .collect())
}

fn run_metadata_generator(workspace_root: &Path, pack_dir: &Path) {
    let status = Command::new("python3")
        .arg(workspace_root.join("tools/generate_pack_metadata.py"))
        .arg("--pack-dir")
        .arg(pack_dir)
        .arg("--components-dir")
        .arg(workspace_root.join("components"))
        .arg("--include-capabilities-cache")
        .arg("--version")
        .arg("test")
        .status()
        .expect("failed to run metadata generator");
    assert!(status.success(), "metadata generator did not exit cleanly");
}

fn manifest_components(manifest_path: &Path) -> Result<Vec<String>> {
    let manifest: Value = serde_json::from_slice(&fs::read(manifest_path)?)?;
    let comps = manifest
        .get("components")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("manifest missing components array"))?;
    Ok(comps
        .iter()
        .filter_map(Value::as_str)
        .map(|s| s.to_string())
        .collect())
}

fn collect_expected_requirements(
    components_dir: &Path,
    component_names: &[String],
) -> Result<BTreeMap<(String, String), Map<String, Value>>> {
    let mut merged: BTreeMap<(String, String), Map<String, Value>> = BTreeMap::new();
    for component in component_names {
        let manifest_path = components_dir
            .join(component)
            .join("component.manifest.json");
        if !manifest_path.exists() {
            continue;
        }
        let manifest: Value = serde_json::from_slice(&fs::read(manifest_path)?)?;
        if let Some(reqs) = manifest
            .get("secret_requirements")
            .and_then(|v| v.as_array())
        {
            for req in reqs {
                let obj = req
                    .as_object()
                    .cloned()
                    .ok_or_else(|| anyhow!("requirement must be an object"))?;
                let name = obj
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("requirement missing name"))?
                    .to_string();
                let scope = obj
                    .get("scope")
                    .and_then(Value::as_str)
                    .unwrap_or("tenant")
                    .to_string();
                merged.entry((name, scope)).or_insert(obj);
            }
        }
    }
    Ok(merged)
}

fn requirement_keys(
    requirements: &[Value],
) -> Result<BTreeMap<(String, String), Map<String, Value>>> {
    let mut merged: BTreeMap<(String, String), Map<String, Value>> = BTreeMap::new();
    for req in requirements {
        let obj = req
            .as_object()
            .cloned()
            .ok_or_else(|| anyhow!("requirement must be an object"))?;
        let name = obj
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("requirement missing name"))?
            .to_string();
        let scope = obj
            .get("scope")
            .and_then(Value::as_str)
            .unwrap_or("tenant")
            .to_string();
        merged.insert((name, scope), obj);
    }
    Ok(merged)
}

#[test]
fn gtpack_contains_secret_requirements_metadata() -> Result<()> {
    let root = workspace_root();
    let pack_source = root.join("packs").join("messaging-telegram");
    let temp = tempdir()?;
    let pack_copy = temp.path().join("messaging-telegram");
    copy_dir(&pack_source, &pack_copy)?;

    run_metadata_generator(&root, &pack_copy);

    let gtpack_path = temp.path().join("messaging-telegram.gtpack");
    build_gtpack(&pack_copy, &gtpack_path)?;

    let manifest_bytes = read_from_gtpack(&gtpack_path, "pack.manifest.json")?;
    let manifest: Value = serde_json::from_slice(&manifest_bytes)?;

    let schema_path = manifest
        .get("config_schema")
        .and_then(|v| v.get("provider_config"))
        .and_then(|v| v.get("path"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("pack manifest missing config schema path"))?;
    assert_eq!(
        schema_path, "schemas/messaging/telegram/public.config.schema.json",
        "unexpected config schema path for messaging-telegram"
    );

    let requirements = manifest
        .get("secret_requirements")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("pack manifest missing secret_requirements"))?;

    assert!(
        !requirements.is_empty(),
        "secret_requirements should be populated for messaging-telegram"
    );

    let components = manifest_components(&pack_copy.join("pack.manifest.json"))?;
    let expected = collect_expected_requirements(&root.join("components"), &components)?;
    let actual = requirement_keys(requirements)?;

    assert_eq!(
        requirements.len(),
        actual.len(),
        "secret_requirements should be deduplicated by name+scope"
    );

    let expected_keys: BTreeSet<_> = expected.keys().cloned().collect();
    let actual_keys: BTreeSet<_> = actual.keys().cloned().collect();
    assert_eq!(
        expected_keys, actual_keys,
        "secret requirement keys should match component manifests"
    );

    for key in expected_keys {
        let expected_req = expected.get(&key).unwrap();
        let actual_req = actual.get(&key).unwrap();
        assert_eq!(
            actual_req.get("description"),
            expected_req.get("description"),
            "description should be preserved for {:?}",
            key
        );
        assert_eq!(
            actual_req.get("example"),
            expected_req.get("example"),
            "example should be preserved for {:?}",
            key
        );
        for field in actual_req.keys() {
            assert!(
                matches!(field.as_str(), "name" | "scope" | "description" | "example"),
                "unexpected field {} in requirement {:?}",
                field,
                key
            );
        }
    }

    let cache = manifest
        .get("capabilities_cache")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("pack manifest missing capabilities_cache"))?;
    assert!(
        !cache.is_empty(),
        "capabilities_cache should include entries when capabilities_v1.json exists"
    );
    for entry in cache {
        let obj = entry
            .as_object()
            .ok_or_else(|| anyhow!("capabilities_cache entry must be object"))?;
        let component = obj
            .get("component")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("capabilities_cache entry missing component"))?;
        assert!(
            components.contains(&component.to_string()),
            "capabilities_cache component {} not in manifest components",
            component
        );
        let path = obj
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("capabilities_cache entry missing path"))?;
        let cache_bytes = read_from_gtpack(&gtpack_path, path)?;
        assert!(
            !cache_bytes.is_empty(),
            "capabilities cache file {} should be present",
            path
        );
    }

    Ok(())
}

#[test]
fn packs_lock_has_digest() -> Result<()> {
    use sha2::{Digest, Sha256};

    let root = workspace_root();
    let lock_path = root.join("packs.lock.json");
    let gtpack_path = root
        .join("dist")
        .join("packs")
        .join("messaging-telegram.gtpack");
    let lock_json: Value = serde_json::from_slice(&std::fs::read(&lock_path)?)?;
    let packs = lock_json
        .get("packs")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("packs.lock.json missing packs array"))?;
    let entry = packs
        .iter()
        .find(|p| p.get("name").and_then(Value::as_str) == Some("messaging-telegram"))
        .ok_or_else(|| anyhow!("packs.lock.json missing messaging-telegram entry"))?;
    assert!(
        !packs
            .iter()
            .any(|p| p.get("name").and_then(Value::as_str) == Some("messaging-provider-bundle")),
        "packs.lock.json should not include messaging-provider-bundle"
    );
    let bundle_path = root
        .join("dist")
        .join("packs")
        .join("messaging-provider-bundle.gtpack");
    assert!(
        !bundle_path.exists(),
        "bundle pack artifact should not exist at {}",
        bundle_path.display()
    );
    let digest = entry
        .get("digest")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("packs.lock.json missing digest"))?;
    let bytes = std::fs::read(&gtpack_path)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let hex = format!("{:x}", hasher.finalize());
    assert_eq!(digest, format!("sha256:{hex}"));
    Ok(())
}

#[test]
fn gtpack_templates_nodes_require_config() -> Result<()> {
    let root = workspace_root();
    let dist_dir = root.join("dist").join("packs");
    let mut packs = Vec::new();
    for entry in fs::read_dir(&dist_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("gtpack") {
            continue;
        }
        if let Some(name) = path.file_name().and_then(|s| s.to_str())
            && name.starts_with("messaging-")
        {
            packs.push(path);
        }
    }
    if packs.is_empty() {
        return Err(anyhow!(
            "no messaging-*.gtpack found under {}",
            dist_dir.display()
        ));
    }

    for pack in packs {
        let flow_entries = list_flow_json_entries(&pack)?;
        let mut missing = Vec::new();
        for flow_entry in flow_entries {
            let data: Value = serde_json::from_slice(&read_from_gtpack(&pack, &flow_entry)?)?;
            let nodes = data
                .get("nodes")
                .and_then(|v| v.as_object())
                .ok_or_else(|| anyhow!("flow {} missing nodes", flow_entry))?;
            for (node_id, node) in nodes {
                let comp_id = node
                    .get("component")
                    .and_then(|v| v.get("id"))
                    .and_then(Value::as_str);
                if comp_id != Some("ai.greentic.component-templates") {
                    continue;
                }
                let mapping = node
                    .get("input")
                    .and_then(|v| v.get("mapping"))
                    .and_then(Value::as_object);
                let has_config = mapping
                    .and_then(|map| map.get("config"))
                    .and_then(Value::as_object)
                    .is_some();
                if !has_config {
                    missing.push(format!("{flow_entry}:{node_id}"));
                }
            }
        }
        assert!(
            missing.is_empty(),
            "missing config in templates nodes for {}: {}",
            pack.display(),
            missing.join(", ")
        );
    }

    Ok(())
}
