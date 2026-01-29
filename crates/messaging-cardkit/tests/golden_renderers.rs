use gsm_core::messaging_card::adaptive::normalizer;
use gsm_core::messaging_card::spec::RenderSpec;
use messaging_cardkit::{CardKit, StaticProfiles, Tier};
use serde_json::Value;
use std::{fs, path::PathBuf, sync::Arc};

fn load_card_fixture(name: &str) -> Value {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests/fixtures/cards");
    path.push(format!("{name}.json"));
    let data = fs::read_to_string(path).expect("card fixture missing");
    serde_json::from_str(&data).expect("invalid card json")
}

fn load_renderer_payload(provider: &str) -> Value {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests/fixtures/renderers");
    path.push(format!("{provider}.json"));
    let file = fs::File::open(path).expect("renderer fixture missing");
    serde_json::from_reader(file).expect("invalid renderer json")
}

#[test]
fn renderers_match_golden_payloads() {
    let card = load_card_fixture("basic");
    let ir = normalizer::ac_to_ir(&card).expect("normalize card");
    let spec = RenderSpec::Card(Box::new(ir));
    let profiles = Arc::new(
        StaticProfiles::builder()
            .default_tier(Tier::Premium)
            .build(),
    );
    let kit = CardKit::new(profiles);

    let expectations = [
        ("slack", false),
        ("teams", false),
        ("webchat", false),
        ("webex", false),
        ("telegram", true),
        ("whatsapp", true),
    ];

    for (provider, downgraded) in expectations {
        let response = kit
            .render_with_spec(provider, &spec)
            .unwrap_or_else(|err| panic!("{provider} render failed: {err}"));
        let expected_payload = load_renderer_payload(provider);
        assert_eq!(response.payload, expected_payload, "{provider} payload");
        assert!(response.warnings.is_empty(), "{provider} warnings");
        assert_eq!(
            response.preview.downgraded, downgraded,
            "{provider} downgraded flag"
        );
    }
}
