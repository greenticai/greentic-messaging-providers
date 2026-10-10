//! Shared attachment fixtures (master plan C6) and contract C1 conformance.

use std::collections::HashMap;
use std::path::PathBuf;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use greentic_interfaces_wasmtime::host_helpers::v1::http_client;
use provider_tests::harness::{TestHostState, default_secret_values};
use provider_tests::universal::{ProviderHarness, ProviderId, provider_spec};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// One native payload (C6 name) and the provider output it must produce.
const CHANNELS: &[(&str, &str)] = &[
    ("slack", "slack-file-share.json"),
    ("telegram", "telegram-photo.json"),
    ("whatsapp", "whatsapp-image.json"),
    ("webex", "webex-files.json"),
    ("teams", "teams-attachment.json"),
    ("webchat", "webchat-upload.json"),
];

/// Per-channel copies that must stay byte-identical to the C6 files.
const COPIES: &[(&str, &str)] = &[
    ("slack/inbound/file_share.json", "slack-file-share.json"),
    ("telegram/inbound/photo_caption.json", "telegram-photo.json"),
    ("whatsapp/inbound/image_message.json", "whatsapp-image.json"),
    (
        "teams/inbound/file_download_info.json",
        "teams-attachment.json",
    ),
];

const ALLOWED_MIME: &[&str] = &[
    "image/jpeg",
    "image/png",
    "image/gif",
    "image/webp",
    "text/plain",
    "text/markdown",
    "text/csv",
    "application/json",
    "application/pdf",
];

/// Exact host (or `*.suffix`) the provider may name for `bearer`/`public`.
const URL_HOSTS: &[(&str, &str)] = &[
    ("slack", "files.slack.com"),
    ("webex", "webexapis.com"),
    ("teams", "*.sharepoint.com"),
];

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn dir() -> PathBuf {
    fixtures().join("attachments-v1")
}

fn load(name: &str) -> Value {
    let text = std::fs::read_to_string(dir().join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn expected(channel: &str) -> Value {
    load(&format!("expected-{channel}.json"))
}

fn json_files() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir())
        .expect("dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".json"))
        .collect();
    names.sort();
    names
}

#[test]
fn checksums_cover_every_fixture_and_match() {
    let listed: HashMap<String, String> = std::fs::read_to_string(dir().join("CHECKSUMS.sha256"))
        .expect("CHECKSUMS.sha256")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let (sum, name) = l.split_once("  ").expect("`<sha256>  <name>` line");
            (name.to_string(), sum.to_string())
        })
        .collect();
    let files = json_files();
    assert_eq!(
        {
            let mut l: Vec<_> = listed.keys().cloned().collect();
            l.sort();
            l
        },
        files,
        "CHECKSUMS.sha256 must list exactly the .json fixtures"
    );
    for name in files {
        let bytes = std::fs::read(dir().join(&name)).expect("read");
        let sum: String = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(
            listed[&name], sum,
            "{name} changed without CHECKSUMS.sha256"
        );
    }
}

#[test]
fn every_channel_has_a_native_and_an_expected_fixture() {
    for (channel, native) in CHANNELS {
        assert!(dir().join(native).exists(), "{native} missing");
        assert!(
            dir().join(format!("expected-{channel}.json")).exists(),
            "expected-{channel}.json missing"
        );
    }
}

#[test]
fn channel_copies_are_byte_identical() {
    for (copy, c6) in COPIES {
        assert_eq!(
            std::fs::read(fixtures().join(copy)).expect("copy"),
            std::fs::read(dir().join(c6)).expect("c6"),
            "{copy} differs from attachments-v1/{c6}"
        );
    }
}

#[test]
fn no_fixture_carries_a_credential() {
    let secrets: Vec<String> = default_secret_values()
        .into_values()
        .map(|v| String::from_utf8_lossy(&v).to_ascii_lowercase())
        .collect();
    for name in json_files() {
        let text = std::fs::read_to_string(dir().join(&name))
            .expect("read")
            .to_ascii_lowercase();
        for needle in ["xoxb-", "bearer ", "/bot", "access_token", "authorization"]
            .into_iter()
            .chain(secrets.iter().map(String::as_str))
        {
            assert!(!text.contains(needle), "{name} contains {needle}");
        }
    }
}

fn host_of(url: &str) -> &str {
    url.strip_prefix("https://")
        .unwrap_or_else(|| panic!("not https: {url}"))
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
}

fn host_allowed(channel: &str, url: &str) -> bool {
    let host = host_of(url);
    URL_HOSTS.iter().any(|(c, rule)| {
        *c == channel
            && match rule.strip_prefix('*') {
                Some(suffix) => host.ends_with(suffix) && host.len() > suffix.len(),
                None => host == *rule,
            }
    })
}

/// C1 shape of one provider envelope before the host runs.
fn assert_before_host(channel: &str, envelope: &Value) {
    let atts = envelope["attachments"].as_array().expect("attachments");
    let refs = envelope["attachment_fetch"]
        .as_array()
        .expect("attachment_fetch");
    assert_eq!(
        atts.len(),
        refs.len(),
        "{channel}: lists not index-parallel"
    );
    assert!(atts.len() <= 5, "{channel}: over 5 attachments");
    for (a, r) in atts.iter().zip(refs) {
        assert!(a["url"].is_null(), "{channel}: url must be absent or null");
        let mime = a["mime_type"].as_str().expect("mime_type");
        let kind = r["kind"].as_str().expect("kind");
        match kind {
            "bearer" => {
                assert!(ALLOWED_MIME.contains(&mime) || mime == "application/octet-stream");
                let key = r["secret_key"].as_str().expect("secret_key");
                assert!(
                    key.chars().all(|c| c.is_ascii_uppercase() || c == '_'),
                    "{key}"
                );
                assert!(
                    host_allowed(channel, r["url"].as_str().expect("url")),
                    "{channel}: {r}"
                );
                assert!(a["content"].is_null());
            }
            "public" => {
                assert!(ALLOWED_MIME.contains(&mime), "{channel}: {mime}");
                assert!(
                    host_allowed(channel, r["url"].as_str().expect("url")),
                    "{channel}: {r}"
                );
                assert!(a["content"].is_null());
            }
            "telegram_file" | "whatsapp_media" => {
                assert!(ALLOWED_MIME.contains(&mime), "{channel}: {mime}");
                let id = r["file_id"]
                    .as_str()
                    .or(r["media_id"].as_str())
                    .expect("id");
                assert!(!id.is_empty() && id.len() <= 256);
                assert!(
                    id.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                );
                assert!(a["content"].is_null());
            }
            "inline" => {
                assert!(ALLOWED_MIME.contains(&mime), "{channel}: {mime}");
                let b64 = a["content"]["data_base64"]
                    .as_str()
                    .expect("content.data_base64");
                let bytes = STANDARD.decode(b64).expect("base64");
                assert!(bytes.len() <= 10 * 1024 * 1024);
                assert_eq!(a["size_bytes"].as_u64(), Some(bytes.len() as u64));
                assert_eq!(
                    r.as_object().map(|o| o.len()),
                    Some(1),
                    "inline carries only kind"
                );
            }
            "none" => assert_eq!(channel, "webex", "only Webex carries cards"),
            other => panic!("{channel}: unknown kind {other}"),
        }
    }
}

#[test]
fn every_expected_provider_output_follows_contract_c1() {
    let before = load("inbound-before-host.json");
    assert_before_host(
        "slack",
        &json!({
            "attachments": before["attachments"],
            "attachment_fetch": before["extensions"]["attachment_fetch"],
        }),
    );
    for (channel, _) in CHANNELS {
        let envelopes = expected(channel)["envelopes"]
            .as_array()
            .expect("envelopes")
            .clone();
        assert!(
            envelopes
                .iter()
                .any(|e| !e["attachments"].as_array().is_none_or(Vec::is_empty)),
            "{channel}: no attachment at all"
        );
        for envelope in envelopes {
            if envelope["attachments"]
                .as_array()
                .is_some_and(|a| !a.is_empty())
            {
                assert_before_host(channel, &envelope);
            } else {
                assert!(
                    envelope["attachment_fetch"].is_null(),
                    "{channel}: {envelope}"
                );
            }
        }
    }
}

/// Contract C1: `artifact://` + lowercase hex of 32 bytes (64 characters).
fn is_canonical_artifact_id(value: &Value) -> bool {
    value
        .as_str()
        .and_then(|u| u.strip_prefix("artifact://"))
        .is_some_and(|hex| {
            hex.len() == 64
                && hex
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
}

fn is_sha256_hex(value: &Value) -> bool {
    value.as_str().is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

/// sha256 of greentic-designer `tests/fixtures/attachments-v1/outbound-tool-result.json`,
/// the C6 owner copy; this repo carries it byte for byte.
const CANONICAL_OUTBOUND_TOOL_RESULT_SHA256: &str =
    "50cfa9823558d97b300e0d95b0d031ba201267e3305e11e3228379961bcd2fd8";

#[test]
fn outbound_tool_result_is_the_designer_canonical_copy() {
    let bytes = std::fs::read(dir().join("outbound-tool-result.json")).expect("read");
    let sum: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(sum, CANONICAL_OUTBOUND_TOOL_RESULT_SHA256);
}

#[test]
fn after_host_urls_are_artifact_references_and_parallel() {
    let v = load("inbound-after-host.json");
    let atts = v["attachments"].as_array().expect("attachments");
    let arts = v["extensions"]["artifacts"].as_array().expect("artifacts");
    assert_eq!(atts.len(), arts.len());
    for a in atts {
        assert!(is_canonical_artifact_id(&a["url"]), "{a}");
    }
    for art in arts {
        assert!(matches!(art["kind"].as_str(), Some("image" | "document")));
        assert!(is_sha256_hex(&art["sha256"]), "{art}");
        assert!(art["text_ref"].is_null() || is_canonical_artifact_id(&art["text_ref"]));
    }
}

#[test]
fn outbound_tool_result_has_the_c5_shape() {
    let v = load("outbound-tool-result.json");
    assert_eq!(v["ok"], true);
    assert!(is_canonical_artifact_id(&v["artifact"]["id"]), "{v}");
    assert!(v["artifact"]["mime_type"].is_string() && v["artifact"]["name"].is_string());
}

/// What a provider emitted, reduced to the C1 fields.
fn project(events: &[Value]) -> Value {
    let envelopes: Vec<Value> = events
        .iter()
        .map(|e| {
            json!({
                "attachments": e.get("attachments").cloned().unwrap_or(json!([])),
                "attachment_fetch": e["extensions"]["attachment_fetch"],
                "attachments_dropped": e["metadata"]["attachments_dropped"],
            })
        })
        .collect();
    json!({ "envelopes": envelopes })
}

fn ingest(id: ProviderId, state: TestHostState, headers: &Value, body: &Value) -> Vec<Value> {
    let mut harness = ProviderHarness::new_with_state(provider_spec(id), state).expect("harness");
    let headers: Vec<Value> = headers
        .as_object()
        .map(|m| {
            m.iter()
                .map(|(k, v)| json!({"name": k, "value": v.as_str().unwrap_or_default()}))
                .collect()
        })
        .unwrap_or_default();
    let input = json!({
        "method": "POST",
        "path": "/webhook",
        "query": null,
        "headers": headers,
        "body_b64": STANDARD.encode(serde_json::to_vec(body).expect("body")),
        "route_hint": null,
        "binding_id": null,
        "config": null,
    });
    let out = harness
        .call("ingest_http", serde_json::to_vec(&input).expect("input"))
        .expect("ingest_http");
    let out: Value = serde_json::from_slice(&out).expect("out");
    let text = out.to_string().to_ascii_lowercase();
    for secret in default_secret_values().into_values() {
        let secret = String::from_utf8_lossy(&secret).to_ascii_lowercase();
        assert!(!text.contains(&secret), "provider output leaks {secret}");
    }
    out["events"].as_array().cloned().unwrap_or_default()
}

fn assert_provider(channel: &str, id: ProviderId, state: TestHostState) {
    let native = load(
        CHANNELS
            .iter()
            .find(|(c, _)| *c == channel)
            .expect("channel")
            .1,
    );
    let events = ingest(id, state, &native["headers"], &native["body"]);
    assert_eq!(
        project(&events),
        expected(channel),
        "{channel}: provider output differs from attachments-v1/expected-{channel}.json"
    );
}

#[test]
fn slack_emits_the_expected_attachments() {
    assert_provider(
        "slack",
        ProviderId::Slack,
        TestHostState::with_default_secrets(),
    );
}

#[test]
fn telegram_emits_the_expected_attachments() {
    assert_provider(
        "telegram",
        ProviderId::Telegram,
        TestHostState::with_default_secrets(),
    );
}

#[test]
fn whatsapp_emits_the_expected_attachments() {
    assert_provider(
        "whatsapp",
        ProviderId::Whatsapp,
        TestHostState::with_default_secrets(),
    );
}

#[test]
fn webex_emits_the_expected_attachments() {
    let message = serde_json::to_vec(&load("webex-files.json")["message"]).expect("message");
    let handler = move |req: http_client::RequestV1_1| {
        if req.method.eq_ignore_ascii_case("GET") && req.url.ends_with("/messages/msg-1") {
            Ok(http_client::ResponseV1_1 {
                status: 200,
                headers: Vec::new(),
                body: Some(message.clone()),
            })
        } else {
            Ok(http_client::ResponseV1_1 {
                status: 404,
                headers: Vec::new(),
                body: None,
            })
        }
    };
    let state = TestHostState::with_secrets(default_secret_values(), handler);
    assert_provider("webex", ProviderId::Webex, state);
}
