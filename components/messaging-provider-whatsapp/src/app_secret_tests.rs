use super::*;
use bindings::exports::greentic::component::qa::Guest as QaGuest;
use bindings::exports::greentic::component::qa::Mode;

const APP_SECRET: &str = "app-secret-under-test";

fn apply(mode: Mode, answers: Value) -> Value {
    let out = <Component as QaGuest>::apply_answers(mode, canonical_cbor_bytes(&answers));
    decode_cbor(&out).expect("decode apply output")
}

fn base_answers() -> Value {
    json!({
        "phone_number_id": "123",
        "public_base_url": "https://example.com",
        "whatsapp_token": "token-a"
    })
}

#[test]
fn apply_answers_maps_the_app_secret_to_its_secret_key() {
    let mut answers = base_answers();
    answers["whatsapp_app_secret"] = json!(APP_SECRET);
    for mode in [Mode::Setup, Mode::Default] {
        let out = apply(mode, answers.clone());
        assert_eq!(out["ok"], Value::Bool(true));
        assert_eq!(
            out["secrets_patch"]["set"]["WHATSAPP_APP_SECRET"],
            Value::String(APP_SECRET.to_string())
        );
    }
}

#[test]
fn upgrade_sets_the_app_secret_only_when_answered() {
    let existing = json!({
        "enabled": true,
        "phone_number_id": "123",
        "public_base_url": "https://example.com",
        "api_base_url": "https://graph.facebook.com",
        "api_version": "v19.0"
    });
    let out = apply(
        Mode::Upgrade,
        json!({"existing_config": existing, "whatsapp_app_secret": APP_SECRET}),
    );
    assert_eq!(out["ok"], Value::Bool(true));
    assert_eq!(
        out["secrets_patch"]["set"]["WHATSAPP_APP_SECRET"],
        Value::String(APP_SECRET.to_string())
    );

    let out = apply(
        Mode::Upgrade,
        json!({"existing_config": existing, "api_version": "v20.0"}),
    );
    assert_eq!(out["ok"], Value::Bool(true));
    assert!(
        out["secrets_patch"]["set"]
            .get("WHATSAPP_APP_SECRET")
            .is_none()
    );
}

#[test]
fn the_app_secret_is_optional() {
    for answers in [base_answers(), {
        let mut blank = base_answers();
        blank["whatsapp_app_secret"] = json!("   ");
        blank
    }] {
        let out = apply(Mode::Setup, answers);
        assert_eq!(out["ok"], Value::Bool(true));
        assert!(
            out["secrets_patch"]["set"]
                .get("WHATSAPP_APP_SECRET")
                .is_none()
        );
    }
}

#[test]
fn the_app_secret_never_reaches_the_config_or_describe() {
    let mut answers = base_answers();
    answers["whatsapp_app_secret"] = json!(APP_SECRET);
    let out = apply(Mode::Setup, answers);
    let config = serde_json::to_string(&out["config"]).expect("config json");
    assert!(!config.contains(APP_SECRET));
    assert!(!config.contains("app_secret"));

    let describe = serde_json::to_string(&build_describe_payload()).expect("describe json");
    assert!(!describe.contains(APP_SECRET));
}
