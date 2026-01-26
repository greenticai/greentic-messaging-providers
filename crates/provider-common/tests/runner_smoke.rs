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
    pack_id: &str,
    config: serde_json::Value,
    answers: serde_json::Value,
    dry_run: bool,
) -> Result<()> {
    let answers_json = serde_json::to_string(&answers)?;
    let input = json!({
        "id": pack_id,
        "tenant": "operator",
        "team": "operator",
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
            tenant_id: Some("operator".to_string()),
            team_id: Some("operator".to_string()),
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

struct SetupFlowTest {
    pack_file: &'static str,
    pack_id: &'static str,
    config: serde_json::Value,
    answers: serde_json::Value,
}

#[test]
fn runner_desktop_setup_default_smoke() -> Result<()> {
    let root = workspace_root();
    let packs_dir = root.join("dist").join("packs");

    let tests = vec![
        SetupFlowTest {
            pack_file: "messaging-dummy.gtpack",
            pack_id: "messaging-dummy",
            config: json!({}),
            answers: json!({}),
        },
        SetupFlowTest {
            pack_file: "messaging-email.gtpack",
            pack_id: "messaging-email",
            config: json!({
                "host": "smtp.example.com",
                "port": 587,
                "username": "mailer",
                "from_address": "noreply@example.com",
                "tls_mode": "starttls",
                "password": "email-secret",
            }),
            answers: json!({
                "host": "smtp.example.com",
                "port": 587,
                "username": "mailer",
                "from_address": "noreply@example.com",
                "tls_mode": "starttls",
                "password": "email-secret",
            }),
        },
        SetupFlowTest {
            pack_file: "messaging-slack.gtpack",
            pack_id: "messaging-slack",
            config: json!({
                "public_base_url": "https://example.com",
                "default_channel": "#general",
                "bot_token": "slack-test-token",
            }),
            answers: json!({
                "public_base_url": "https://example.com",
                "default_channel": "#general",
                "bot_token": "slack-test-token",
            }),
        },
        SetupFlowTest {
            pack_file: "messaging-teams.gtpack",
            pack_id: "messaging-teams",
            config: json!({
                "tenant_id": "tenant-123",
                "client_id": "client-123",
                "public_base_url": "https://example.com",
                "team_id": "team-xyz",
                "client_secret": "teams-secret",
            }),
            answers: json!({
                "tenant_id": "tenant-123",
                "client_id": "client-123",
                "public_base_url": "https://example.com",
                "team_id": "team-xyz",
                "client_secret": "teams-secret",
            }),
        },
        SetupFlowTest {
            pack_file: "messaging-telegram.gtpack",
            pack_id: "messaging-telegram",
            config: json!({
                "public_base_url": "https://example.com",
                "default_chat_id": "12345",
            }),
            answers: json!({
                "public_base_url": "https://example.com",
                "default_chat_id": "12345",
                "bot_token": "telegram-test-token",
            }),
        },
        SetupFlowTest {
            pack_file: "messaging-webchat.gtpack",
            pack_id: "messaging-webchat",
            config: json!({
                "public_base_url": "https://example.com",
                "mode": "webhook",
                "ingress_path": "/webhooks/webchat",
            }),
            answers: json!({
                "public_base_url": "https://example.com",
                "mode": "webhook",
                "ingress_path": "/webhooks/webchat",
            }),
        },
        SetupFlowTest {
            pack_file: "messaging-webex.gtpack",
            pack_id: "messaging-webex",
            config: json!({
                "public_base_url": "https://example.com",
                "default_room_id": "room-123",
            }),
            answers: json!({
                "public_base_url": "https://example.com",
                "default_room_id": "room-123",
                "bot_token": "webex-test-token",
            }),
        },
        SetupFlowTest {
            pack_file: "messaging-whatsapp.gtpack",
            pack_id: "messaging-whatsapp",
            config: json!({
                "phone_number_id": "12345",
                "business_account_id": "business-abc",
                "public_base_url": "https://example.com",
                "access_token": "whatsapp-secret",
            }),
            answers: json!({
                "phone_number_id": "12345",
                "business_account_id": "business-abc",
                "public_base_url": "https://example.com",
                "access_token": "whatsapp-secret",
            }),
        },
    ];

    for test in tests {
        let pack_path = packs_dir.join(test.pack_file);
        if !pack_path.exists() {
            return Err(anyhow!(
                "missing gtpack {}; expected under {}",
                pack_path.display(),
                packs_dir.display()
            ));
        }
        run_setup_default(
            &pack_path,
            test.pack_id,
            test.config.clone(),
            test.answers.clone(),
            true,
        )?;
    }

    Ok(())
}

#[test]
fn runner_desktop_abi_compat_smoke() -> Result<()> {
    let root = workspace_root();
    let packs_dir = root.join("dist").join("packs");
    let webex = packs_dir.join("messaging-webex.gtpack");

    if !webex.exists() {
        return Err(anyhow!(
            "missing gtpack; expected webex under {}",
            packs_dir.display()
        ));
    }

    let result = run_setup_default(
        &webex,
        "messaging-webex",
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
    );
    result.map_err(|err| {
        let msg = err.to_string();
        if msg.contains("greentic:component/node@") {
            anyhow!("runner ABI mismatch for {}: {msg}", webex.display())
        } else {
            err
        }
    })
}
