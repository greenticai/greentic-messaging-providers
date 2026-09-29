//! `send_typing` is advertised exactly where the platform shows a native typing
//! indicator. Slack, Webex, email, dummy and Teams Graph mode have none:
//! advertising it there would fail every turn or tempt a visible placeholder.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;

const TYPING_PROVIDER_TYPES: &[&str] = &[
    "messaging.webchat",
    "messaging.webchat-gui",
    "messaging.3aigent-gui",
    "messaging.telegram.bot",
    "messaging.whatsapp.cloud",
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn only_typing_capable_providers_declare_send_typing() -> Result<()> {
    let packs = workspace_root().join("packs");
    let mut seen = HashSet::new();
    for entry in fs::read_dir(&packs).context("packs dir")? {
        let dir = entry?.path();
        let manifest_path = dir.join("pack.manifest.json");
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
        let yaml = fs::read_to_string(dir.join("pack.yaml")).unwrap_or_default();
        for provider in providers {
            let provider_type = provider["provider_type"].as_str().unwrap_or_default();
            let declared = provider["ops"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|op| op.as_str() == Some("send_typing"));
            let allowed = TYPING_PROVIDER_TYPES.contains(&provider_type);
            assert_eq!(
                declared,
                allowed,
                "{}: {provider_type} send_typing declared={declared}, allowed={allowed}",
                dir.display()
            );
            assert_eq!(
                yaml.contains("- send_typing"),
                declared,
                "{}: pack.yaml and pack.manifest.json disagree on send_typing",
                dir.display()
            );
            if declared {
                seen.insert(provider_type.to_string());
            }
        }
    }
    for provider_type in TYPING_PROVIDER_TYPES {
        assert!(
            seen.contains(*provider_type),
            "{provider_type} lost send_typing"
        );
    }
    Ok(())
}

#[test]
fn the_teams_bot_pack_declares_send_typing() -> Result<()> {
    let script = fs::read_to_string(workspace_root().join("messaging-teams/build_pack.sh"))?;
    let block = script
        .split("provider_type: messaging.teams\n")
        .nth(1)
        .and_then(|rest| rest.split("config_schema_ref").next())
        .context("messaging.teams provider block")?;
    assert!(
        block.contains("- send_typing"),
        "bot pack must advertise send_typing"
    );
    Ok(())
}
