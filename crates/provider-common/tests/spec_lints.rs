use std::fs;
use std::path::PathBuf;

use anyhow::{Result, anyhow};
use glob::glob;
use serde_yaml_bw::Value;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

#[test]
fn all_packs_have_setup_spec() -> Result<()> {
    let root = workspace_root();
    let packs_dir = root.join("packs");
    for entry in fs::read_dir(&packs_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let spec = path.join("assets").join("setup.yaml");
        if !spec.exists() {
            return Err(anyhow!("missing setup spec at {}", spec.display()));
        }
    }
    Ok(())
}

#[test]
fn specs_parse() -> Result<()> {
    let root = workspace_root();
    let pattern = root.join("packs").join("*/assets/setup.yaml");
    let mut found = false;
    for entry in glob(pattern.to_str().unwrap())? {
        let path = entry?;
        let contents = fs::read_to_string(&path)?;
        let _: Value = serde_yaml_bw::from_str(&contents)?;
        found = true;
    }
    if !found {
        return Err(anyhow!("no setup.yaml files found under packs/"));
    }
    Ok(())
}

#[test]
fn secret_questions_not_inlined_in_titles() -> Result<()> {
    let root = workspace_root();
    let pattern = root.join("packs").join("*/assets/setup.yaml");
    for entry in glob(pattern.to_str().unwrap())? {
        let path = entry?;
        let contents = fs::read_to_string(&path)?;
        let value: Value = serde_yaml_bw::from_str(&contents)?;
        let questions = value
            .get("questions")
            .and_then(Value::as_sequence)
            .cloned()
            .unwrap_or_default();
        for question in questions {
            let secret = question
                .get("secret")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if !secret {
                continue;
            }
            let title = question.get("title").and_then(Value::as_str).unwrap_or("");
            if title.to_ascii_lowercase().contains("token:") {
                return Err(anyhow!(
                    "secret question title should not include token value hints in {}",
                    path.display()
                ));
            }
        }
    }
    Ok(())
}
