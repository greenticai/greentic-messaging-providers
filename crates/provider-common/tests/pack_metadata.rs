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
fn webchat_gui_pack_declares_provider_routes_and_static_assets() -> Result<()> {
    let root = workspace_root();
    let pack_dir = root.join("packs").join("messaging-webchat-gui");
    let manifest_path = pack_dir.join("pack.manifest.json");
    let manifest: Value = serde_json::from_slice(&fs::read(&manifest_path)?)?;

    let provider = manifest
        .get("extensions")
        .and_then(|ext| ext.get("greentic.provider-extension.v1"))
        .and_then(|ext| ext.get("inline"))
        .and_then(|inline| inline.get("providers"))
        .and_then(Value::as_array)
        .and_then(|providers| providers.first())
        .ok_or_else(|| anyhow!("webchat-gui manifest missing provider extension entry"))?;
    assert_eq!(
        provider.get("provider_type").and_then(Value::as_str),
        Some("messaging.webchat-gui")
    );
    assert_eq!(
        provider
            .get("runtime")
            .and_then(|rt| rt.get("component_ref"))
            .and_then(Value::as_str),
        Some("messaging-provider-webchat-gui")
    );
    assert_eq!(
        provider.get("config_schema_ref").and_then(Value::as_str),
        Some("schemas/messaging/webchat-gui/public.config.schema.json")
    );

    let provider_routes = manifest
        .get("extensions")
        .and_then(|ext| ext.get("messaging.provider_routes.v1"))
        .and_then(|ext| ext.get("inline"))
        .ok_or_else(|| anyhow!("webchat-gui manifest missing provider routes"))?;
    assert_eq!(
        provider_routes
            .get("backend_base_path")
            .and_then(Value::as_str),
        Some("/v1/messaging/webchat/{tenant}")
    );
    let routes = provider_routes
        .get("routes")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("webchat-gui manifest missing backend routes array"))?;
    assert!(
        routes.iter().any(|route| {
            route.get("path").and_then(Value::as_str)
                == Some("/v1/messaging/webchat/{tenant}/token")
        }),
        "expected token route in provider routes"
    );
    assert!(
        routes.iter().any(|route| {
            route.get("path").and_then(Value::as_str)
                == Some("/v1/messaging/webchat/{tenant}/v3/directline/{*path}")
        }),
        "expected directline route prefix in provider routes"
    );

    let static_route = manifest
        .get("extensions")
        .and_then(|ext| ext.get("greentic.static-routes.v1"))
        .and_then(|ext| ext.get("inline"))
        .and_then(|inline| inline.get("routes"))
        .and_then(Value::as_array)
        .and_then(|routes| routes.first())
        .ok_or_else(|| anyhow!("webchat-gui manifest missing static route"))?;
    assert_eq!(
        static_route.get("public_path").and_then(Value::as_str),
        Some("/v1/web/webchat/{tenant}")
    );
    assert_eq!(
        static_route.get("source_root").and_then(Value::as_str),
        Some("assets/webchat-gui")
    );
    assert_eq!(
        static_route
            .get("scope")
            .and_then(|scope| scope.get("tenant"))
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        static_route
            .get("scope")
            .and_then(|scope| scope.get("team"))
            .and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        static_route.get("index_file").and_then(Value::as_str),
        Some("index.html")
    );
    assert_eq!(
        static_route.get("spa_fallback").and_then(Value::as_str),
        Some("index.html")
    );

    let staged_component = pack_dir.join("components/messaging-provider-webchat-gui.wasm");
    assert!(
        staged_component.exists(),
        "expected staged component artifact at {}",
        staged_component.display()
    );

    Ok(())
}

#[test]
fn webchat_gui_pack_contains_runtime_bootstrap_and_bundled_assets() -> Result<()> {
    let root = workspace_root();
    let asset_root = root
        .join("packs")
        .join("messaging-webchat-gui")
        .join("assets/webchat-gui");

    for rel in [
        "index.html",
        "404.html",
        "runtime-bootstrap.js",
        "config/product.json",
    ] {
        let path = asset_root.join(rel);
        assert!(path.exists(), "missing asset {}", path.display());
    }

    let bootstrap = fs::read_to_string(asset_root.join("runtime-bootstrap.js"))?;
    assert!(
        bootstrap.contains("/v1/web/webchat/"),
        "runtime bootstrap should resolve tenant from the GUI path"
    );
    assert!(
        bootstrap.contains("/v1/messaging/webchat/"),
        "runtime bootstrap should point to provider-scoped backend routes"
    );
    assert!(
        bootstrap.contains("__WEBCHAT_BACKEND_BASE__"),
        "runtime bootstrap should expose the backend base"
    );

    let mut has_js_bundle = false;
    let mut has_css_bundle = false;
    for entry in fs::read_dir(asset_root.join("assets"))? {
        let path = entry?.path();
        if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
            has_js_bundle |= name.ends_with(".js");
            has_css_bundle |= name.ends_with(".css");
        }
    }
    assert!(has_js_bundle, "expected packaged JS bundle");
    assert!(has_css_bundle, "expected packaged CSS bundle");

    Ok(())
}
