use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

fn workspace_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn command_exists(bin: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {}", bin))
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

#[test]
fn packgen_generates_pack_and_doctor_passes() {
    if env::var("RUN_PACKGEN_TESTS").is_err() {
        return;
    }
    if !command_exists("greentic-pack") || !command_exists("greentic-flow") {
        eprintln!("greentic-pack or greentic-flow not available; skipping packgen test");
        return;
    }

    let root = workspace_root();
    let temp_dir = TempDir::new().expect("tempdir");
    let spec_dir = root.join("specs").join("providers");
    let out_dir = temp_dir.path().join("out");
    let dist_dir = out_dir.join("dist");
    fs::create_dir_all(&dist_dir).expect("create dist dir");

    let status = Command::new(env!("CARGO_BIN_EXE_greentic-messaging-packgen"))
        .current_dir(&root)
        .args(["generate-all", "--spec-dir"])
        .arg(&spec_dir)
        .args(["--out"])
        .arg(&out_dir)
        .status()
        .expect("run packgen generate-all");
    assert!(status.success(), "packgen generate-all failed");

    let mut pack_dirs = Vec::new();
    for entry in fs::read_dir(&out_dir).expect("read out dir") {
        let entry = entry.expect("entry");
        let path = entry.path();
        if path.is_dir() && path.join("pack.yaml").exists() {
            pack_dirs.push(path);
        }
    }
    pack_dirs.sort();

    for pack_out in pack_dirs {
        let pack_id = load_pack_id(&pack_out.join("pack.yaml"));
        let pack_path = dist_dir.join(format!("{pack_id}.gtpack"));
        let status = Command::new("greentic-pack")
            .current_dir(&pack_out)
            .args(["build", "--no-update", "--in", ".", "--gtpack-out"])
            .arg(&pack_path)
            .args(["--secrets-req", "secret-requirements.json"])
            .status()
            .expect("run greentic-pack build");
        assert!(status.success(), "greentic-pack build failed");

        let status = Command::new("greentic-pack")
            .args(["doctor", "--validate", "--pack"])
            .arg(&pack_path)
            .status()
            .expect("run greentic-pack doctor");
        assert!(status.success(), "greentic-pack doctor failed");
    }
}

fn load_pack_id(path: &Path) -> String {
    let contents = fs::read_to_string(path).expect("read pack.yaml");
    let value: serde_yaml::Value = serde_yaml::from_str(&contents).expect("parse pack.yaml");
    value
        .get("pack_id")
        .and_then(|v| v.as_str())
        .expect("pack_id in pack.yaml")
        .to_string()
}
