//! Setup inputs the host needs to verify inbound requests. Each must stay
//! optional: a required one would refuse every existing deployment.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use serde_json::Value as JsonValue;
use serde_yaml_bw::Value;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

fn setup_question(setup_path: &Path, name: &str) -> Result<Value> {
    let spec: Value = serde_yaml_bw::from_str(&fs::read_to_string(setup_path)?)?;
    spec.get("questions")
        .and_then(Value::as_sequence)
        .and_then(|questions| {
            questions
                .iter()
                .find(|q| q.get("name").and_then(Value::as_str) == Some(name))
        })
        .cloned()
        .ok_or_else(|| anyhow!("{} has no question {name}", setup_path.display()))
}

fn flag(question: &Value, key: &str) -> bool {
    question.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn whatsapp_pack() -> PathBuf {
    workspace_root().join("packs").join("messaging-whatsapp")
}

#[test]
fn whatsapp_app_secret_is_an_optional_secret_question_without_default() -> Result<()> {
    let question = setup_question(
        &whatsapp_pack().join("assets").join("setup.yaml"),
        "whatsapp_app_secret",
    )?;
    assert!(!flag(&question, "required"), "must stay optional");
    assert!(flag(&question, "secret"), "must be marked secret");
    assert!(question.get("default").is_none(), "a secret has no default");
    assert!(question.get("visible_if").is_none());
    Ok(())
}

fn maps_answer_to_secret(secrets_out: &[JsonValue]) -> bool {
    secrets_out.iter().any(|mapping| {
        mapping.get("answer_key").and_then(JsonValue::as_str) == Some("whatsapp_app_secret")
            && mapping.get("secret_key").and_then(JsonValue::as_str) == Some("WHATSAPP_APP_SECRET")
    })
}

#[test]
fn whatsapp_setup_contract_maps_the_app_secret() -> Result<()> {
    let pack_yaml: JsonValue =
        serde_yaml_bw::from_str(&fs::read_to_string(whatsapp_pack().join("pack.yaml"))?)?;
    let manifest: JsonValue = serde_json::from_str(&fs::read_to_string(
        whatsapp_pack().join("pack.manifest.json"),
    )?)?;
    for (label, doc) in [("pack.yaml", &pack_yaml), ("pack.manifest.json", &manifest)] {
        let secrets_out = doc
            .pointer("/extensions/greentic.provider-extension.v1/inline/providers/0/setup_contract/secrets_out")
            .and_then(JsonValue::as_array)
            .ok_or_else(|| anyhow!("{label} has no secrets_out"))?;
        assert!(
            maps_answer_to_secret(secrets_out),
            "{label} must map whatsapp_app_secret to WHATSAPP_APP_SECRET"
        );
    }
    Ok(())
}

#[test]
fn whatsapp_app_secret_requirement_is_declared_optional_everywhere() -> Result<()> {
    let pack = whatsapp_pack();
    let manifest: JsonValue =
        serde_json::from_str(&fs::read_to_string(pack.join("pack.manifest.json"))?)?;
    let manifest_reqs = manifest
        .get("secret_requirements")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| anyhow!("manifest has no secret_requirements"))?;
    let entry = manifest_reqs
        .iter()
        .find(|r| r.get("name").and_then(JsonValue::as_str) == Some("WHATSAPP_APP_SECRET"))
        .ok_or_else(|| anyhow!("manifest does not declare WHATSAPP_APP_SECRET"))?;
    assert_eq!(entry.get("required"), Some(&JsonValue::Bool(false)));

    // greentic-setup and greentic-start default an absent `required` to true.
    for rel in [
        "secret-requirements.json",
        ".secret_requirements.json",
        ".packc/secret-requirements.json",
    ] {
        let reqs: JsonValue = serde_json::from_str(&fs::read_to_string(pack.join(rel))?)?;
        let entry = reqs
            .as_array()
            .and_then(|reqs| {
                reqs.iter().find(|r| {
                    r.get("key").and_then(JsonValue::as_str) == Some("WHATSAPP_APP_SECRET")
                })
            })
            .ok_or_else(|| anyhow!("{rel} does not declare WHATSAPP_APP_SECRET"))?;
        assert_eq!(
            entry.get("required"),
            Some(&JsonValue::Bool(false)),
            "{rel} must mark WHATSAPP_APP_SECRET optional"
        );
    }
    Ok(())
}
