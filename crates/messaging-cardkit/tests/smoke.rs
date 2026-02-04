use messaging_cardkit::{CardKit, RenderIntent, StaticProfiles, Tier};
use serde_json::json;
use std::sync::Arc;

#[test]
fn smoke_text_only() {
    let kit = CardKit::new(Arc::new(
        StaticProfiles::builder().default_tier(Tier::Basic).build(),
    ));
    let response = kit
        .render("basic", &json!({ "kind": "standard", "text": "hello" }))
        .expect("should render");
    assert_eq!(response.intent, RenderIntent::Card);
    assert_eq!(response.preview.tier, Tier::Basic);
    assert!(!response.downgraded);
}
