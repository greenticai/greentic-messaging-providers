use std::collections::BTreeMap;

use anyhow::{Context, Result};
use greentic_types::messaging::universal_dto::{
    EncodeInV1, HttpOutV1, RenderPlanInV1, SendPayloadResultV1,
};
use provider_tests::universal::{
    EncodeResponse, PROVIDERS, ProviderHarness, ProviderId, RenderPlanResponse, SchemaCoreHarness,
    build_envelope, decode_challenge, http_input_from_fixture, load_http_fixture, provider_spec,
    send_payload_in,
};
use serde_json::Value;

// Compile-time WIT contract validation — these bindings are not called at
// runtime but verify that the WASM components match the expected world shape.
macro_rules! provider_bindings {
    ($module:ident, $path:literal, $world:literal) => {
        mod $module {
            wasmtime::component::bindgen!({
                path: $path,
                world: $world,
            });
        }
    };
}

provider_bindings!(
    slack_bindings,
    "../../components/messaging-provider-slack/wit/messaging-provider-slack",
    "component-v0-v6-v0"
);
provider_bindings!(
    telegram_bindings,
    "../../components/messaging-provider-telegram/wit/messaging-provider-telegram",
    "component-v0-v6-v0"
);
provider_bindings!(
    teams_bindings,
    "../../components/messaging-provider-teams/wit/messaging-provider-teams",
    "component-v0-v6-v0"
);
provider_bindings!(
    webchat_bindings,
    "../../components/messaging-provider-webchat/wit/messaging-provider-webchat",
    "component-v0-v6-v0"
);
provider_bindings!(
    webex_bindings,
    "../../components/messaging-provider-webex/wit/messaging-provider-webex",
    "component-v0-v6-v0"
);
provider_bindings!(
    whatsapp_bindings,
    "../../components/messaging-provider-whatsapp/wit/messaging-provider-whatsapp",
    "component-v0-v6-v0"
);
provider_bindings!(
    email_bindings,
    "../../components/messaging-provider-email/wit/messaging-provider-email",
    "component-v0-v6-v0"
);
provider_bindings!(
    dummy_bindings,
    "../../components/messaging-provider-dummy/wit/messaging-provider-dummy",
    "component-v0-v6-v0"
);

#[test]
fn universal_ops_conformance() -> Result<()> {
    for spec in PROVIDERS {
        if spec.skip_universal_ops {
            eprintln!(
                "[provider-tests] skipping universal_ops_conformance for {:?}",
                spec.id
            );
            continue;
        }
        let mut harness = ProviderHarness::new(spec)?;

        // --- ingest ---
        if spec.ingest_supported {
            let fixture = load_http_fixture(spec.fixture)?;
            let http_in = http_input_from_fixture(fixture);
            let ingest_bytes = serde_json::to_vec(&http_in)?;
            let ingest_out = harness.call("ingest_http", ingest_bytes)?;
            let ingest_value: Value = serde_json::from_slice(&ingest_out)?;
            if let Ok(http_out) = serde_json::from_value::<HttpOutV1>(ingest_value.clone()) {
                assert_eq!(http_out.status, 200, "{:?} should support ingest", spec.id);
                assert!(
                    !http_out.events.is_empty(),
                    "{:?} ingest should emit events",
                    spec.id
                );
            } else {
                assert_eq!(
                    ingest_value.get("ok").and_then(Value::as_bool),
                    Some(true),
                    "{:?} legacy ingest should return ok=true: {}",
                    spec.id,
                    ingest_value
                );
                assert!(
                    ingest_value.get("event").is_some() || ingest_value.get("events").is_some(),
                    "{:?} legacy ingest should emit event(s): {}",
                    spec.id,
                    ingest_value
                );
            }
        }

        // --- challenge ---
        if let Some(challenge_fixture) = spec.challenge_fixture {
            let challenge_input = http_input_from_fixture(load_http_fixture(challenge_fixture)?);
            let challenge_bytes = serde_json::to_vec(&challenge_input)?;
            let challenge_out = harness.call("ingest_http", challenge_bytes)?;
            let out_value: Value = serde_json::from_slice(&challenge_out)?;
            let out: HttpOutV1 = serde_json::from_value(out_value.clone()).unwrap_or(HttpOutV1 {
                status: if out_value
                    .get("ok")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    200
                } else {
                    400
                },
                headers: Vec::new(),
                body_b64: out_value
                    .get("body_b64")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                events: Vec::new(),
            });
            assert_eq!(out.status, 200, "challenge should return 200");
            if let Some(expected) = spec.challenge_response {
                let value = if let Some(v) = decode_challenge(&out) {
                    v
                } else {
                    out_value
                        .get("body")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string()
                };
                assert_eq!(value, expected, "challenge response mismatch");
            }
        }

        // --- render_plan ---
        let message = build_envelope(spec.id);
        let plan_in = RenderPlanInV1 {
            message: message.clone(),
            metadata: BTreeMap::new(),
        };
        let plan_bytes = serde_json::to_vec(&plan_in)?;
        let plan_out_bytes = harness.call("render_plan", plan_bytes)?;
        let plan_value: Value = serde_json::from_slice(&plan_out_bytes)?;
        if let Ok(plan_response) = serde_json::from_value::<RenderPlanResponse>(plan_value.clone())
        {
            assert!(plan_response.ok, "{:?} render_plan failed", spec.id);
            assert!(
                plan_response.plan.is_some(),
                "{:?} render_plan missing plan",
                spec.id
            );
        } else {
            assert!(
                plan_value
                    .get("ok")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                    || plan_value.get("plan").is_some(),
                "{:?} legacy render_plan output invalid: {}",
                spec.id,
                plan_value
            );
        }

        // --- encode ---
        let encode_in = EncodeInV1 {
            message: message.clone(),
            plan: plan_in,
        };
        let encode_bytes = serde_json::to_vec(&encode_in)?;
        let encode_out = harness.call("encode", encode_bytes)?;
        let encode_value: Value = serde_json::from_slice(&encode_out)?;
        if let Ok(encode_response) = serde_json::from_value::<EncodeResponse>(encode_value.clone())
        {
            assert!(encode_response.ok, "{:?} encode failed", spec.id);
            assert!(
                encode_response.payload.is_some(),
                "{:?} encode missing payload",
                spec.id
            );
        } else {
            assert!(
                encode_value
                    .get("ok")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                    || encode_value.get("payload").is_some(),
                "{:?} legacy encode output invalid: {}",
                spec.id,
                encode_value
            );
        }

        // --- send_payload ---
        let send_bytes = send_payload_in(spec)?;
        let send_out = harness.call("send_payload", send_bytes)?;
        let send_value: Value = serde_json::from_slice(&send_out)?;
        if let Ok(send_result) = serde_json::from_value::<SendPayloadResultV1>(send_value.clone()) {
            assert!(
                !send_result.retryable,
                "{:?} send_payload should not retry",
                spec.id
            );
        } else {
            assert!(
                !send_value
                    .get("retryable")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                "{:?} legacy send_payload should not be retryable: {}",
                spec.id,
                send_value
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// QA ops via schema-core-api invoke() path
// ---------------------------------------------------------------------------

/// Verify that `invoke("qa-spec", {"mode":"setup"})` through the schema-core-api
/// JSON path returns a valid QA spec with questions for every provider.
#[test]
fn qa_spec_via_invoke_returns_valid_json() -> Result<()> {
    for spec in PROVIDERS {
        let mut harness = SchemaCoreHarness::new(spec)?;
        let input = serde_json::to_vec(&serde_json::json!({"mode": "setup"}))?;
        let out = harness.call("qa-spec", input)?;
        let value: Value = serde_json::from_slice(&out).context(format!(
            "{:?} qa-spec response should be valid JSON",
            spec.id
        ))?;

        assert_eq!(
            value.get("mode").and_then(Value::as_str),
            Some("setup"),
            "{:?} qa-spec should echo mode=setup",
            spec.id
        );

        let questions = value
            .get("questions")
            .and_then(Value::as_array)
            .unwrap_or_else(|| {
                panic!(
                    "{:?} qa-spec should contain a questions array, got: {}",
                    spec.id, value
                )
            });
        assert!(
            !questions.is_empty(),
            "{:?} qa-spec questions should not be empty",
            spec.id
        );

        for (i, q) in questions.iter().enumerate() {
            assert!(
                q.get("id").and_then(Value::as_str).is_some(),
                "{:?} qa-spec question[{}] missing \"id\" field: {}",
                spec.id,
                i,
                q
            );
            // label is an I18nText object with a "key" field, not a plain string.
            let label = q.get("label").unwrap_or_else(|| {
                panic!(
                    "{:?} qa-spec question[{}] missing \"label\" field: {}",
                    spec.id, i, q
                )
            });
            assert!(
                label.get("key").and_then(Value::as_str).is_some(),
                "{:?} qa-spec question[{}] label should have an i18n \"key\": {}",
                spec.id,
                i,
                label
            );
            // kind must be present
            assert!(
                q.get("kind").is_some(),
                "{:?} qa-spec question[{}] missing \"kind\" field: {}",
                spec.id,
                i,
                q
            );
        }

        eprintln!(
            "[provider-tests] {:?} qa-spec OK ({} questions)",
            spec.id,
            questions.len()
        );
    }
    Ok(())
}

/// Verify that `invoke("apply-answers", ...)` through the schema-core-api JSON
/// path returns `ok: true` and a config object for the Dummy provider.
#[test]
fn apply_answers_via_invoke_returns_config() -> Result<()> {
    let spec = provider_spec(ProviderId::Dummy);
    let mut harness = SchemaCoreHarness::new(spec)?;

    let input = serde_json::to_vec(&serde_json::json!({
        "mode": "setup",
        "answers": {
            "api_token": "test-token",
            "endpoint_url": "https://example.com"
        },
        "current_config": {}
    }))?;

    let out = harness.call("apply-answers", input)?;
    let value: Value = serde_json::from_slice(&out).context(format!(
        "{:?} apply-answers response should be valid JSON",
        spec.id
    ))?;

    assert_eq!(
        value.get("ok").and_then(Value::as_bool),
        Some(true),
        "{:?} apply-answers should return ok=true, got: {}",
        spec.id,
        value
    );

    let config = value.get("config").unwrap_or_else(|| {
        panic!(
            "{:?} apply-answers should contain a config object, got: {}",
            spec.id, value
        )
    });
    assert!(
        config.is_object(),
        "{:?} apply-answers config should be an object, got: {}",
        spec.id,
        config
    );

    eprintln!(
        "[provider-tests] {:?} apply-answers OK, config={}",
        spec.id, config
    );
    Ok(())
}

/// Verify that `invoke("i18n-keys", {})` through the schema-core-api JSON path
/// returns a non-empty array of strings for every provider.
#[test]
fn i18n_keys_via_invoke_returns_array() -> Result<()> {
    for spec in PROVIDERS {
        let mut harness = SchemaCoreHarness::new(spec)?;
        let input = serde_json::to_vec(&serde_json::json!({}))?;
        let out = harness.call("i18n-keys", input)?;
        let value: Value = serde_json::from_slice(&out).context(format!(
            "{:?} i18n-keys response should be valid JSON",
            spec.id
        ))?;

        let keys = value.as_array().unwrap_or_else(|| {
            panic!(
                "{:?} i18n-keys should return a JSON array, got: {}",
                spec.id, value
            )
        });
        assert!(
            !keys.is_empty(),
            "{:?} i18n-keys should return a non-empty array",
            spec.id
        );

        for (i, key) in keys.iter().enumerate() {
            assert!(
                key.as_str().is_some(),
                "{:?} i18n-keys[{}] should be a string, got: {}",
                spec.id,
                i,
                key
            );
        }

        eprintln!(
            "[provider-tests] {:?} i18n-keys OK ({} keys)",
            spec.id,
            keys.len()
        );
    }
    Ok(())
}
