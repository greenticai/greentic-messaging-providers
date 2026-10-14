use crate::setup::{API_KEY_SECRET, WEBHOOK_SECRET_SECRET, apply_answers};
use serde_json::json;

const KEY: &str = "0123456789abcdef-api-key";
const HOOK_SECRET: &str = "whsec_0123456789";

#[test]
fn setup_with_only_an_api_key_is_valid_and_has_no_webhook() {
    let out = apply_answers("setup", &json!({"api_key": KEY}));
    assert!(out.ok, "{:?}", out.error);
    let config = out.config.expect("config");
    assert!(config.enabled);
    assert_eq!(config.webhook_url, None);
    let patch = out.secrets_patch.expect("patch");
    assert_eq!(patch.set.get(API_KEY_SECRET).map(String::as_str), Some(KEY));
    assert!(!patch.set.contains_key(WEBHOOK_SECRET_SECRET));
}

#[test]
fn secrets_travel_only_in_the_patch_never_in_the_config() {
    let out = apply_answers(
        "setup",
        &json!({"api_key": KEY, "webhook_url": "https://hooks.example.com/in", "webhook_secret": HOOK_SECRET}),
    );
    assert!(out.ok);
    let config_json = serde_json::to_string(&out.config).expect("json");
    assert!(!config_json.contains(KEY) && !config_json.contains(HOOK_SECRET));
    let patch = out.secrets_patch.expect("patch");
    assert_eq!(patch.set.len(), 2);
    assert_eq!(
        patch.set.get(WEBHOOK_SECRET_SECRET).map(String::as_str),
        Some(HOOK_SECRET)
    );
}

#[test]
fn the_api_key_is_required_on_setup_and_must_not_be_short() {
    assert!(!apply_answers("setup", &json!({})).ok);
    let short = apply_answers("setup", &json!({"api_key": "short"}));
    assert!(!short.ok);
    assert!(short.error.expect("error").contains("at least"));
}

#[test]
fn the_webhook_url_must_be_https() {
    for bad in [
        "http://hooks.example.com",
        "ftp://x",
        "https://",
        "hooks.example.com",
    ] {
        let out = apply_answers(
            "setup",
            &json!({"api_key": KEY, "webhook_url": bad, "webhook_secret": HOOK_SECRET}),
        );
        assert!(!out.ok, "{bad} must be refused");
    }
}

#[test]
fn a_webhook_url_without_a_secret_is_refused_on_setup() {
    let out = apply_answers(
        "setup",
        &json!({"api_key": KEY, "webhook_url": "https://h.example.com/x"}),
    );
    assert!(!out.ok);
    assert!(out.error.expect("error").contains("webhook_secret"));
}

#[test]
fn upgrade_keeps_what_it_is_not_told_about() {
    let out = apply_answers(
        "upgrade",
        &json!({"existing_config": {"enabled": true, "webhook_url": "https://h.example.com/x"}, "enabled": false}),
    );
    assert!(out.ok, "{:?}", out.error);
    let config = out.config.expect("config");
    assert!(!config.enabled);
    assert_eq!(
        config.webhook_url.as_deref(),
        Some("https://h.example.com/x")
    );
    assert!(
        out.secrets_patch.is_none(),
        "no secret was supplied, none is rewritten"
    );
}

#[test]
fn remove_deletes_both_secrets() {
    let out = apply_answers("remove", &json!({}));
    assert!(out.ok);
    let patch = out.secrets_patch.expect("patch");
    assert!(patch.delete.contains(&API_KEY_SECRET.to_string()));
    assert!(patch.delete.contains(&WEBHOOK_SECRET_SECRET.to_string()));
    assert!(out.remove.expect("remove").remove_all);
}

#[test]
fn the_pack_setup_fixtures_match_what_apply_answers_produces() {
    let dir = format!(
        "{}/../../packs/messaging-api/fixtures",
        env!("CARGO_MANIFEST_DIR")
    );
    let read = |name: &str| -> serde_json::Value {
        let text = std::fs::read_to_string(format!("{dir}/{name}")).expect("fixture");
        serde_json::from_str(&text).expect("json")
    };
    let input = read("setup.input.json");
    let expected = read("setup.expected.plan.json");
    let mut answers = json!({"api_key": input["secrets"]["API_CHANNEL_API_KEY"]});
    if let Some(enabled) = input["config"].get("enabled") {
        answers["enabled"] = enabled.clone();
    }
    let out = apply_answers(input["mode"].as_str().expect("mode"), &answers);
    assert!(out.ok, "{:?}", out.error);
    let config = serde_json::to_value(out.config).expect("config");
    assert_eq!(config, expected["config_patch"]);
    let patch = serde_json::to_value(out.secrets_patch).expect("patch");
    assert_eq!(patch, expected["secrets_patch"]);
}
