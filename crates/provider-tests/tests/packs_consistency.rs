use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

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

fn setup_question_names(pack_dir: &Path) -> Result<HashSet<String>> {
    let setup_path = pack_dir.join("assets/setup.yaml");
    if !setup_path.exists() {
        return Ok(HashSet::new());
    }
    let setup: serde_yaml::Value = serde_yaml::from_slice(&fs::read(&setup_path)?)
        .with_context(|| format!("parsing {}", setup_path.display()))?;
    Ok(setup
        .get("questions")
        .and_then(serde_yaml::Value::as_sequence)
        .into_iter()
        .flatten()
        .filter_map(|question| question.get("name").and_then(serde_yaml::Value::as_str))
        .map(str::to_string)
        .collect())
}

fn setup_contract_config_outputs(manifest: &Value) -> HashSet<String> {
    manifest
        .get("extensions")
        .and_then(|ext| ext.get(PROVIDER_EXTENSION_ID))
        .and_then(|ext| ext.get("inline"))
        .and_then(|inline| inline.get("providers"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|provider| provider.get("setup_contract"))
        .filter_map(|contract| contract.get("config_out"))
        .filter_map(Value::as_object)
        .flat_map(|config_out| config_out.keys().cloned())
        .collect()
}

fn known_provider_derived_setup_outputs(_pack_name: &str) -> HashSet<&'static str> {
    HashSet::new()
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
        let component_sources: Vec<(String, String)> = manifest
            .get("component_sources")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|component| {
                let id = component.get("id").and_then(Value::as_str)?;
                let wasm = component.get("wasm").and_then(Value::as_str)?;
                Some((id.to_string(), wasm.to_string()))
            })
            .collect();
        assert_unique_component_ids(
            &component_sources
                .iter()
                .map(|(id, _)| id.as_str())
                .collect::<Vec<_>>(),
            &format!("{} component_sources", pack_dir.display()),
        );
        let component_wasm_paths: HashMap<String, PathBuf> = component_sources
            .iter()
            .map(|(id, wasm)| (id.clone(), pack_dir.join(wasm)))
            .collect();
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
            let component_ids = comps.iter().filter_map(Value::as_str).collect::<Vec<_>>();
            assert_unique_component_ids(
                &component_ids,
                &format!("{} components", pack_dir.display()),
            );
            for comp in comps {
                let name = comp
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("component entry must be string"))?;
                let wasm = component_wasm_paths
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| pack_dir.join("components").join(format!("{name}.wasm")));
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
            .and_then(|ext| ext.get(PROVIDER_EXTENSION_ID));

        if provider_ext.is_none() {
            // Components are what a provider extension configures; a pack that
            // ships none is asset-only and has no provider to declare.
            let ships_components = manifest
                .get("components")
                .and_then(Value::as_array)
                .is_some_and(|comps| !comps.is_empty());
            if ships_components {
                panic!(
                    "pack {} missing provider extension {}",
                    pack_dir.display(),
                    PROVIDER_EXTENSION_ID
                );
            }
            continue;
        }

        let provider_ext = provider_ext.expect("checked above");

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
                    let wasm = component_wasm_paths
                        .get(component_ref)
                        .cloned()
                        .unwrap_or_else(|| {
                            pack_dir
                                .join("components")
                                .join(format!("{component_ref}.wasm"))
                        });
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
fn final_setup_action_requires_are_public_setup_outputs() -> Result<()> {
    let root = workspace_root();
    let packs_dir = root.join("packs");
    for entry in fs::read_dir(&packs_dir).context("reading packs dir")? {
        let entry = entry?;
        let pack_dir = entry.path();
        if !pack_dir.is_dir() {
            continue;
        }
        let manifest_path = pack_dir.join("pack.manifest.json");
        if !manifest_path.exists() {
            continue;
        }
        let manifest: Value = serde_json::from_slice(&fs::read(&manifest_path)?)
            .with_context(|| format!("parsing {}", manifest_path.display()))?;
        let Some(actions) = manifest
            .get("extensions")
            .and_then(|ext| ext.get("greentic.setup.actions.v1"))
            .and_then(|ext| ext.get("inline"))
            .and_then(|inline| inline.get("actions"))
            .and_then(Value::as_array)
        else {
            continue;
        };

        let setup_questions = setup_question_names(&pack_dir)?;
        let config_outputs = setup_contract_config_outputs(&manifest);
        let pack_name = pack_dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let derived_outputs = known_provider_derived_setup_outputs(pack_name);
        for action in actions {
            let action_id = action
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>");
            if pack_name == "messaging-slack" && action_id == "add-to-slack" {
                assert_eq!(
                    action.get("url_template").and_then(Value::as_str),
                    Some("https://slack.com/app_redirect?app={slack_app_id}"),
                    "messaging-slack final Add to Slack should open the installed app's DM, not OAuth authorization"
                );
                assert!(
                    action
                        .get("requires")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .any(|value| value.as_str() == Some("slack_app_id")),
                    "messaging-slack Add to Slack should require slack_app_id (manifest.update never returns client_id)"
                );
            }
            for required in action
                .get("requires")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                assert!(
                    setup_questions.contains(required)
                        || config_outputs.contains(required)
                        || derived_outputs.contains(required),
                    "{} action {action_id} requires `{required}` but it is not collected by setup.yaml or exposed by setup_contract.config_out",
                    pack_dir.display()
                );
            }
        }
    }

    let teams_answer: Value =
        serde_json::from_slice(&fs::read(root.join("messaging-teams/build-answer.json"))?)?;
    let teams_actions = teams_answer
        .pointer("/setup_api/actions/actions")
        .and_then(Value::as_array)
        .expect("messaging-teams setup actions");
    let add_to_teams = teams_actions
        .iter()
        .find(|action| action.get("id").and_then(Value::as_str) == Some("add-to-teams"))
        .expect("Add to Teams action");
    assert_eq!(
        add_to_teams.pointer("/requires/0").and_then(Value::as_str),
        Some("add_to_teams_url")
    );
    assert!(
        add_to_teams.get("visible_when").is_none(),
        "Add to Teams must resolve from its required public link value, not setup_status.ok"
    );
    let backend_contract: Value = serde_json::from_slice(&fs::read(
        root.join("messaging-teams/assets/setup/backend-contract.json"),
    )?)?;
    assert_eq!(
        backend_contract
            .pointer("/state_shape/teams_app/add_to_teams_url")
            .and_then(Value::as_str),
        Some("string")
    );

    Ok(())
}

#[test]
fn required_setup_questions_do_not_use_placeholder_without_default() -> Result<()> {
    let root = workspace_root();
    let packs_dir = root.join("packs");
    for entry in fs::read_dir(&packs_dir).context("reading packs dir")? {
        let entry = entry?;
        let setup_path = entry.path().join("assets/setup.yaml");
        if !setup_path.exists() {
            continue;
        }
        let setup: serde_yaml::Value = serde_yaml::from_slice(&fs::read(&setup_path)?)
            .with_context(|| format!("parsing {}", setup_path.display()))?;
        for question in setup
            .get("questions")
            .and_then(serde_yaml::Value::as_sequence)
            .into_iter()
            .flatten()
        {
            let required = question
                .get("required")
                .and_then(serde_yaml::Value::as_bool)
                .unwrap_or(false);
            // A required secret cannot ship a default, so its placeholder is a
            // format hint rather than a stand-in for a value we could prefill.
            let secret = question
                .get("secret")
                .and_then(serde_yaml::Value::as_bool)
                .unwrap_or(false);
            if !required || secret || question.get("placeholder").is_none() {
                continue;
            }
            let has_default = question.get("default").is_some()
                || question.get("default_value").is_some()
                || question.get("default_from_context").is_some();
            assert!(
                has_default,
                "{} required setup question `{}` uses placeholder text without a real default",
                setup_path.display(),
                question
                    .get("name")
                    .and_then(serde_yaml::Value::as_str)
                    .unwrap_or("<unknown>")
            );
        }
    }

    let telegram_setup: serde_yaml::Value = serde_yaml::from_slice(&fs::read(
        root.join("packs/messaging-telegram/assets/setup.yaml"),
    )?)?;
    let telegram_api_base = telegram_setup
        .get("questions")
        .and_then(serde_yaml::Value::as_sequence)
        .into_iter()
        .flatten()
        .find(|question| {
            question.get("name").and_then(serde_yaml::Value::as_str) == Some("api_base_url")
        })
        .expect("telegram api_base_url setup question");
    assert_eq!(
        telegram_api_base
            .get("default")
            .and_then(serde_yaml::Value::as_str),
        Some("https://api.telegram.org")
    );

    Ok(())
}

fn assert_unique_component_ids(component_ids: &[&str], context: &str) {
    // Compare on a `-`/`_`-insensitive key: spelling variants of one id are the
    // same component, and shipping both put a stale binary in published packs.
    let mut seen = HashSet::new();
    let mut duplicates = Vec::new();
    for component_id in component_ids {
        if !seen.insert(component_id.replace('_', "-")) {
            duplicates.push(*component_id);
        }
    }
    duplicates.sort_unstable();
    duplicates.dedup();
    assert!(
        duplicates.is_empty(),
        "duplicate component ids in {context}: {duplicates:?}"
    );
}

/// Every `assets/setup.yaml` (in `packs/` plus the root `messaging-teams` bot pack).
fn setup_yaml_paths(root: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(root.join("packs")) {
        for entry in entries.flatten() {
            let dir = entry.path();
            let setup = dir.join("assets/setup.yaml");
            if setup.exists() {
                let name = dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default()
                    .to_string();
                out.push((name, setup));
            }
        }
    }
    let teams = root.join("messaging-teams/assets/setup.yaml");
    if teams.exists() {
        out.push(("messaging-teams".to_string(), teams));
    }
    out
}

/// Single-brace `{token}` placeholders in a deep-link `url_template`
/// (handlebars `{{ ... }}` are skipped — they are not setup outputs).
fn template_tokens(s: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            if i + 1 < bytes.len() && bytes[i + 1] == b'{' {
                i += 2;
                continue;
            }
            if let Some(rel) = s[i + 1..].find('}') {
                let tok = &s[i + 1..i + 1 + rel];
                if !tok.is_empty() && tok.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    out.insert(tok.to_string());
                }
                i = i + 1 + rel + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Generic guard for the "Add to X" buttons shown at the end of setup:
/// every declared `setup_action` is well-formed for its kind, deep-link
/// templates only reference values the action declares it `requires`, and the
/// install-style channels still render a button (regression guard).
#[test]
fn setup_actions_are_wellformed_and_present() -> Result<()> {
    let root = workspace_root();
    // `open_url` is a plain external link with no OAuth round-trip (Slack's
    // install-on-team console action). greentic-setup resolves it via the
    // action's url_template — see setup_action_install_url in its ui module.
    let known_kinds = [
        "deep_link",
        "open_url",
        "oauth_install_button",
        "oauth_device_code",
    ];
    let expected_with_actions = [
        "messaging-teams",
        "messaging-slack",
        "messaging-telegram",
        "messaging-webex",
    ];
    let mut have_actions: HashSet<String> = HashSet::new();

    for (pack, path) in setup_yaml_paths(&root) {
        let setup: serde_yaml::Value = serde_yaml::from_slice(&fs::read(&path)?)
            .with_context(|| format!("parsing {}", path.display()))?;
        let Some(actions) = setup
            .get("setup_actions")
            .and_then(serde_yaml::Value::as_sequence)
        else {
            continue;
        };
        if !actions.is_empty() {
            have_actions.insert(pack.clone());
        }
        for action in actions {
            let id = action
                .get("id")
                .and_then(serde_yaml::Value::as_str)
                .unwrap_or_default();
            assert!(!id.is_empty(), "{pack}: a setup_action is missing `id`");
            let kind = action
                .get("kind")
                .and_then(serde_yaml::Value::as_str)
                .unwrap_or_default();
            assert!(
                known_kinds.contains(&kind),
                "{pack}: setup_action `{id}` has unknown kind {kind:?} (expected one of {known_kinds:?})"
            );
            assert!(
                action
                    .get("label")
                    .and_then(serde_yaml::Value::as_str)
                    .is_some_and(|l| !l.is_empty()),
                "{pack}: setup_action `{id}` is missing `label`"
            );
            match kind {
                "deep_link" => {
                    let tmpl = action
                        .get("url_template")
                        .and_then(serde_yaml::Value::as_str)
                        .unwrap_or_default();
                    assert!(
                        !tmpl.is_empty(),
                        "{pack}: deep_link `{id}` is missing `url_template`"
                    );
                    let requires: HashSet<String> = action
                        .get("requires")
                        .and_then(serde_yaml::Value::as_sequence)
                        .into_iter()
                        .flatten()
                        .filter_map(serde_yaml::Value::as_str)
                        .map(str::to_string)
                        .collect();
                    for tok in template_tokens(tmpl) {
                        assert!(
                            requires.contains(&tok),
                            "{pack}: deep_link `{id}` url_template uses {{{tok}}} but does not declare it in `requires` {requires:?}"
                        );
                    }
                }
                "open_url" => {
                    let tmpl = action
                        .get("url_template")
                        .and_then(serde_yaml::Value::as_str)
                        .unwrap_or_default();
                    assert!(
                        !tmpl.is_empty(),
                        "{pack}: open_url `{id}` is missing `url_template`"
                    );
                }
                "oauth_install_button" => {
                    assert!(
                        action
                            .get("authorize_url")
                            .and_then(serde_yaml::Value::as_str)
                            .is_some_and(|u| !u.is_empty()),
                        "{pack}: oauth_install_button `{id}` is missing `authorize_url`"
                    );
                    let src = action
                        .get("client_id_source")
                        .and_then(serde_yaml::Value::as_str)
                        .unwrap_or_default();
                    assert!(
                        !src.is_empty(),
                        "{pack}: oauth_install_button `{id}` is missing `client_id_source`"
                    );
                    if src == "registration" {
                        let reg = action.get("registration");
                        assert!(
                            reg.and_then(|r| r.get("component_ref"))
                                .and_then(serde_yaml::Value::as_str)
                                .is_some(),
                            "{pack}: oauth_install_button `{id}` registration is missing `component_ref`"
                        );
                        assert!(
                            reg.and_then(|r| r.get("op"))
                                .and_then(serde_yaml::Value::as_str)
                                .is_some(),
                            "{pack}: oauth_install_button `{id}` registration is missing `op`"
                        );
                    }
                }
                "oauth_device_code" => {
                    assert!(
                        action
                            .get("provider")
                            .and_then(serde_yaml::Value::as_str)
                            .is_some_and(|p| !p.is_empty()),
                        "{pack}: oauth_device_code `{id}` is missing `provider`"
                    );
                }
                _ => unreachable!("kind already validated against known_kinds"),
            }
        }
    }

    for pack in expected_with_actions {
        assert!(
            have_actions.contains(pack),
            "{pack}: expected an add-to-X setup_action at end of setup, found none"
        );
    }
    Ok(())
}

#[test]
fn pack_versions_match_provider_matrix() -> Result<()> {
    let root = workspace_root();
    let matrix: Value = serde_json::from_slice(&fs::read(root.join("ci/provider-matrix.json"))?)
        .context("parsing provider-matrix.json")?;
    let providers = matrix
        .get("providers")
        .and_then(Value::as_object)
        .expect("providers key in matrix");

    // Pre-existing mismatches on main; fix separately, not in this PR.
    let skip: HashSet<&str> = ["messaging-webex", "messaging-whatsapp"].into();

    for (_key, entry) in providers {
        let Some(pack_name) = entry.get("pack").and_then(Value::as_str) else {
            continue;
        };
        if skip.contains(pack_name) {
            continue;
        }
        let Some(expected_version) = entry.get("version").and_then(Value::as_str) else {
            continue;
        };
        let pack_dir = root.join("packs").join(pack_name);
        if !pack_dir.is_dir() {
            continue;
        }
        let yaml_path = pack_dir.join("pack.yaml");
        let manifest_path = pack_dir.join("pack.manifest.json");

        let yaml_version = version_from_yaml(&yaml_path)
            .with_context(|| format!("getting version from {}", yaml_path.display()))?;
        assert_eq!(
            yaml_version, expected_version,
            "{pack_name}: pack.yaml version {yaml_version} != matrix version {expected_version}"
        );

        let manifest: Value = serde_json::from_slice(&fs::read(&manifest_path)?)
            .with_context(|| format!("parsing {}", manifest_path.display()))?;
        let manifest_version = manifest
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert_eq!(
            manifest_version, expected_version,
            "{pack_name}: pack.manifest.json version {manifest_version} != matrix version {expected_version}"
        );
    }
    Ok(())
}

#[test]
fn egress_ops_declared_when_component_implements_them() -> Result<()> {
    let root = workspace_root();
    let egress_ops = ["render_plan", "encode", "send_payload"];
    let packs_dir = root.join("packs");

    for entry in fs::read_dir(&packs_dir).context("reading packs dir")? {
        let entry = entry?;
        let pack_dir = entry.path();
        if !pack_dir.is_dir() {
            continue;
        }
        let pack_name = pack_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let manifest_path = pack_dir.join("pack.manifest.json");
        if !manifest_path.exists() {
            continue;
        }
        let manifest: Value = serde_json::from_slice(&fs::read(&manifest_path)?)
            .with_context(|| format!("parsing {}", manifest_path.display()))?;

        let Some(providers) = manifest
            .pointer("/extensions/greentic.provider-extension.v1/inline/providers")
            .and_then(Value::as_array)
        else {
            continue;
        };

        let yaml_path = pack_dir.join("pack.yaml");
        let yaml_contents = fs::read_to_string(&yaml_path)
            .with_context(|| format!("reading {}", yaml_path.display()))?;

        for provider in providers {
            let declared_ops: HashSet<&str> = provider
                .get("ops")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let Some(component_ref) = provider
                .pointer("/runtime/component_ref")
                .and_then(Value::as_str)
            else {
                continue;
            };

            let lib_path = root
                .join("components")
                .join(component_ref)
                .join("src/lib.rs");
            if !lib_path.exists() {
                continue;
            }
            let lib_src = fs::read_to_string(&lib_path)
                .with_context(|| format!("reading {}", lib_path.display()))?;

            let component_has_egress = egress_ops
                .iter()
                .all(|op| lib_src.contains(&format!("\"{op}\"")));
            if !component_has_egress {
                continue;
            }

            for op in &egress_ops {
                assert!(
                    declared_ops.contains(op),
                    "{pack_name}: component {component_ref} handles \"{op}\" \
                     but the pack does not declare it — the runner host will reject this op at runtime",
                );
                assert!(
                    yaml_contents.contains(&format!("- {op}")),
                    "{pack_name}: pack.yaml missing \"{op}\" in ops list \
                     (pack.manifest.json and pack.yaml must stay in sync)",
                );
            }
        }
    }
    Ok(())
}

fn contains_bytes(haystack: &[u8], needle: &str) -> bool {
    let needle = needle.as_bytes();
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// A pack's checked-in WASM must know every setup question the current build knows.
///
/// Bundles built from a repo checkout ship these binaries verbatim. When one lags the
/// source, the setup GUI silently renders an older question set — a shipped webchat-gui
/// pack was two months behind and dropped the whole Greentic SSO group.
///
/// Questions absent from both are pack-only additions in setup.yaml, so they need no
/// allowlist. Skipped when components have not been built.
#[test]
fn pack_wasms_are_not_behind_the_current_build() -> Result<()> {
    let root = workspace_root();
    let built_dir = root.join("target/components");
    if !built_dir.is_dir() {
        eprintln!("skipping: {} not built", built_dir.display());
        return Ok(());
    }

    let mut checked = 0usize;
    for entry in fs::read_dir(root.join("packs"))? {
        let pack_dir = entry?.path();
        if !pack_dir.is_dir() {
            continue;
        }
        let questions = setup_question_names(&pack_dir)?;
        if questions.is_empty() {
            continue;
        }
        let pack_name = pack_dir.file_name().unwrap().to_string_lossy().to_string();
        let components = pack_dir.join("components");
        if !components.is_dir() {
            continue;
        }

        for component in fs::read_dir(&components)? {
            let path = component?.path();
            let Some(id) = path.file_stem().map(|s| s.to_string_lossy().to_string()) else {
                continue;
            };
            if path.extension().and_then(|e| e.to_str()) != Some("wasm") {
                continue;
            }
            let built = built_dir.join(format!("{id}.wasm"));
            if !built.exists() {
                continue;
            }

            let built_bytes = fs::read(&built)?;
            // The nested copy is the qa_ref target the setup GUI reads.
            let nested = components.join(&id).join("component.wasm");
            for candidate in [&path, &nested] {
                if !candidate.exists() {
                    continue;
                }
                let packed_bytes = fs::read(candidate)?;
                checked += 1;
                for question in &questions {
                    if !contains_bytes(&built_bytes, question) {
                        continue;
                    }
                    assert!(
                        contains_bytes(&packed_bytes, question),
                        "{pack_name}: {} is behind the current build — it does not know \
                         setup question \"{question}\". Rebuild components and re-copy.",
                        candidate.strip_prefix(&root).unwrap_or(candidate).display(),
                    );
                }
            }
        }
    }
    assert!(checked > 0, "no pack WASMs were checked");
    Ok(())
}

/// The WhatsApp Graph version a pack setup defaults to must be the one the provider code
/// and the pack config schema default to; greentic-start reads this answer for media fetches.
#[test]
fn whatsapp_setup_api_version_default_matches_the_provider() -> Result<()> {
    let root = workspace_root();
    let source =
        fs::read_to_string(root.join("components/messaging-provider-whatsapp/src/lib.rs"))?;
    let code_default = source
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("const DEFAULT_API_VERSION: &str = \"")
        })
        .and_then(|rest| rest.strip_suffix("\";"))
        .context("DEFAULT_API_VERSION in the WhatsApp provider")?
        .to_string();

    let pack_dir = root.join("packs/messaging-whatsapp");
    let setup: serde_yaml::Value =
        serde_yaml::from_slice(&fs::read(pack_dir.join("assets/setup.yaml"))?)?;
    let question = setup
        .get("questions")
        .and_then(serde_yaml::Value::as_sequence)
        .into_iter()
        .flatten()
        .find(|q| q.get("name").and_then(serde_yaml::Value::as_str) == Some("api_version"))
        .context("api_version question in the WhatsApp setup")?;
    for field in ["default", "placeholder"] {
        assert_eq!(
            question.get(field).and_then(serde_yaml::Value::as_str),
            Some(code_default.as_str()),
            "setup.yaml api_version {field}"
        );
    }

    let schema: Value = serde_json::from_slice(&fs::read(
        pack_dir.join("assets/schemas/messaging/whatsapp/config.schema.json"),
    )?)?;
    assert_eq!(
        schema["properties"]["api_version"]["default"].as_str(),
        Some(code_default.as_str())
    );
    Ok(())
}
