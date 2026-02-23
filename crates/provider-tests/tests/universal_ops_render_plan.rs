use std::collections::BTreeMap;

use anyhow::Result;
use greentic_types::messaging::universal_dto::RenderPlanInV1;
use provider_tests::universal::{PROVIDERS, ProviderHarness, ProviderId, build_envelope};
use serde_json::{Value, json};

/// Sample AC card with a title, body text, image, and action.
fn sample_adaptive_card() -> Value {
    json!({
        "type": "AdaptiveCard",
        "version": "1.5",
        "body": [
            { "type": "TextBlock", "text": "Order Confirmation", "weight": "bolder", "size": "large" },
            { "type": "TextBlock", "text": "Your order #12345 has been confirmed." },
            { "type": "Image", "url": "https://example.com/order.png", "size": "medium" },
            {
                "type": "ColumnSet",
                "columns": [
                    { "type": "Column", "items": [{ "type": "TextBlock", "text": "Item" }] },
                    { "type": "Column", "items": [{ "type": "TextBlock", "text": "$49.99" }] }
                ]
            }
        ],
        "actions": [
            { "type": "Action.OpenUrl", "title": "View Order", "url": "https://example.com/order/12345" }
        ]
    })
}

/// AC-capable providers should return TierA/TierB with AC in attachments.
const AC_CAPABLE: &[ProviderId] = &[ProviderId::Teams, ProviderId::Webex, ProviderId::Webchat];

/// Non-AC providers should return TierC/TierD with text extracted from the AC
/// and an "adaptive_card_downsampled" warning.
const NON_AC: &[ProviderId] = &[
    ProviderId::Slack,
    ProviderId::Telegram,
    ProviderId::Whatsapp,
    ProviderId::Email,
];

#[test]
fn render_plan_with_adaptive_card() -> Result<()> {
    for &id in AC_CAPABLE.iter().chain(NON_AC.iter()) {
        let spec = PROVIDERS
            .iter()
            .find(|s| s.id == id)
            .expect("provider spec");
        eprintln!("[provider-tests] render_plan_with_ac for {:?}", spec.id);
        let mut harness = ProviderHarness::new(spec)?;

        let mut message = build_envelope(id);
        let ac_string = serde_json::to_string(&sample_adaptive_card()).expect("serialize AC");
        message
            .metadata
            .insert("adaptive_card".to_string(), ac_string);

        let plan_in = RenderPlanInV1 {
            message,
            metadata: BTreeMap::new(),
        };
        let plan_bytes = serde_json::to_vec(&plan_in)?;
        let plan_out_bytes = harness.call("render_plan", plan_bytes)?;
        let plan_value: Value = serde_json::from_slice(&plan_out_bytes)?;

        assert_eq!(
            plan_value.get("ok").and_then(Value::as_bool),
            Some(true),
            "{:?} render_plan with AC should return ok=true: {}",
            id,
            plan_value
        );

        let plan_out = plan_value
            .get("plan")
            .unwrap_or_else(|| panic!("{id:?} should have plan"));
        let plan_json_str = plan_out
            .get("plan_json")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("{id:?} plan_json should be string"));
        let plan: Value = serde_json::from_str(plan_json_str)
            .unwrap_or_else(|_| panic!("{id:?} plan_json should parse"));

        let tier = plan
            .get("tier")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("{id:?} should have tier"));

        if AC_CAPABLE.contains(&id) {
            // AC-capable providers -> TierA or TierB, AC in attachments
            assert!(
                tier == "TierA" || tier == "TierB",
                "{:?} with AC should be TierA/TierB, got {}",
                id,
                tier
            );
            let attachments = plan
                .get("attachments")
                .and_then(Value::as_array)
                .unwrap_or_else(|| panic!("{id:?} should have attachments array"));
            assert!(
                !attachments.is_empty(),
                "{:?} AC-capable should have AC attachment",
                id
            );
            let first = &attachments[0];
            assert_eq!(
                first.get("type").and_then(Value::as_str),
                Some("AdaptiveCard"),
                "{:?} attachment should be AdaptiveCard",
                id
            );
        } else {
            // Non-AC providers -> TierC or TierD, text extracted, downsampled warning
            assert!(
                tier == "TierC" || tier == "TierD",
                "{:?} without AC support should be TierC/TierD, got {}",
                id,
                tier
            );

            let summary = plan
                .get("summary_text")
                .and_then(Value::as_str)
                .unwrap_or_else(|| panic!("{id:?} should have summary_text"));
            assert!(
                !summary.is_empty(),
                "{:?} summary_text should not be empty",
                id
            );

            let warnings = plan
                .get("warnings")
                .and_then(Value::as_array)
                .unwrap_or_else(|| panic!("{id:?} should have warnings array"));
            let has_downsampled = warnings.iter().any(|w| {
                w.get("code")
                    .and_then(Value::as_str)
                    .is_some_and(|c| c == "adaptive_card_downsampled")
            });
            assert!(
                has_downsampled,
                "{:?} non-AC provider should emit adaptive_card_downsampled warning, got: {:?}",
                id, warnings
            );
        }

        eprintln!(
            "[provider-tests] {:?} render_plan with AC: tier={}, ok",
            id, tier
        );
    }
    Ok(())
}
