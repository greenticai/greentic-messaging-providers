use std::path::PathBuf;

use anyhow::{Result, anyhow};
use greentic_runner_desktop::{RunOptions, TenantContext, run_pack_with_options};
use serde_json::json;
use tempfile::tempdir;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

fn run_setup_default(
    pack_path: &PathBuf,
    config: serde_json::Value,
    answers: serde_json::Value,
    dry_run: bool,
) -> Result<()> {
    let answers_json = serde_json::to_string(&answers)?;
    let input = json!({
        "tenant": "smoke",
        "public_base_url": "https://example.com",
        "config": config,
        "answers": answers,
        "answers_json": answers_json,
        "dry_run": dry_run
    });
    let run_dir = tempdir()?;
    let opts = RunOptions {
        entry_flow: Some("setup_default".to_string()),
        input,
        ctx: TenantContext {
            tenant_id: Some("smoke".to_string()),
            team_id: None,
            user_id: Some("operator".to_string()),
            session_id: None,
        },
        dist_offline: true,
        artifacts_dir: Some(run_dir.path().to_path_buf()),
        ..RunOptions::default()
    };
    run_pack_with_options(pack_path, opts).map_err(|e| anyhow!("{e}"))?;
    Ok(())
}

#[test]
fn runner_desktop_setup_default_smoke() -> Result<()> {
    let root = workspace_root();
    let packs_dir = root.join("dist").join("packs");
    let telegram = packs_dir.join("messaging-telegram.gtpack");
    let webchat = packs_dir.join("messaging-webchat.gtpack");
    let webex = packs_dir.join("messaging-webex.gtpack");

    if !telegram.exists() || !webchat.exists() || !webex.exists() {
        return Err(anyhow!(
            "missing gtpack(s); expected telegram/webchat/webex under {}",
            packs_dir.display()
        ));
    }

    run_setup_default(
        &telegram,
        json!({
            "public_base_url": "https://example.com",
            "default_chat_id": "12345"
        }),
        json!({
            "public_base_url": "https://example.com",
            "default_chat_id": "12345",
            "bot_token": "telegram-test-token"
        }),
        true,
    )?;
    run_setup_default(
        &webchat,
        json!({
            "public_base_url": "https://example.com",
            "mode": "webhook",
            "ingress_path": "/webhooks/webchat"
        }),
        json!({
            "public_base_url": "https://example.com",
            "mode": "webhook",
            "ingress_path": "/webhooks/webchat"
        }),
        true,
    )?;
    run_setup_default(
        &webex,
        json!({
            "public_base_url": "https://example.com",
            "default_room_id": "room-123"
        }),
        json!({
            "public_base_url": "https://example.com",
            "default_room_id": "room-123",
            "bot_token": "webex-test-token"
        }),
        true,
    )?;

    Ok(())
}
