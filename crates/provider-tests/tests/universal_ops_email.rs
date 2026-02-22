use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use greentic_interfaces_wasmtime::host_helpers::v1::http_client;
use greentic_types::messaging::universal_dto::{HttpInV1, HttpOutV1};
use provider_tests::harness::{TestHostState, default_secret_values};
use provider_tests::universal::{ProviderHarness, ProviderId, provider_spec};
use serde_json::{Value, json};

fn email_config_value() -> Value {
    json!({
        "public_base_url": "https://example.com",
        "host": "smtp.example",
        "port": 587,
        "username": "alice",
        "from_address": "alice@example.com",
        "tls_mode": "starttls",
        "graph_tenant_id": "tenant",
        "graph_base_url": "https://graph.microsoft.com/v1.0",
    })
}

fn graph_token_response(token: &str) -> http_client::ResponseV1_1 {
    let body = serde_json::to_vec(&json!({
        "access_token": token,
        "expires_in": 3600
    }))
    .unwrap();
    http_client::ResponseV1_1 {
        status: 200,
        headers: Vec::new(),
        body: Some(body),
    }
}

fn graph_json_response(value: Value) -> http_client::ResponseV1_1 {
    let body = serde_json::to_vec(&value).unwrap();
    http_client::ResponseV1_1 {
        status: 200,
        headers: Vec::new(),
        body: Some(body),
    }
}

fn http_error(msg: &str) -> http_client::HttpClientErrorV1_1 {
    http_client::HttpClientErrorV1_1 {
        code: "GRAPH_ERROR".into(),
        message: msg.to_string(),
    }
}

fn email_secrets() -> std::collections::HashMap<String, Vec<u8>> {
    let mut secrets = default_secret_values();
    secrets.insert("MS_GRAPH_CLIENT_ID".to_string(), b"graph-client".to_vec());
    secrets.insert(
        "MS_GRAPH_CLIENT_SECRET".to_string(),
        b"graph-secret".to_vec(),
    );
    secrets.insert(
        "msgraph:tenant:alice:refresh_token".to_string(),
        b"refresh-token".to_vec(),
    );
    secrets
}

#[test]
fn email_subscription_ops_roundtrip() -> Result<()> {
    let spec = provider_spec(ProviderId::Email);
    let handler = move |req: http_client::RequestV1_1| {
        if req.url.contains("/oauth2/v2.0/token") {
            return Ok(graph_token_response("token-graph"));
        }
        if req.method.eq_ignore_ascii_case("POST") && req.url.ends_with("/subscriptions") {
            return Ok(graph_json_response(json!({
                "id": "sub-email",
                "expirationDateTime": "2025-12-01T00:00:00Z",
                "resource": "/me/mailFolders('Inbox')/messages",
                "changeType": "created",
                "clientState": "client-state"
            })));
        }
        if req.method.eq_ignore_ascii_case("PATCH") {
            return Ok(graph_json_response(json!({
                "id": "sub-email",
                "expirationDateTime": "2025-12-02T00:00:00Z",
            })));
        }
        if req.method.eq_ignore_ascii_case("DELETE") {
            return Ok(http_client::ResponseV1_1 {
                status: 204,
                headers: Vec::new(),
                body: None,
            });
        }
        Err(http_error("unexpected graph request"))
    };
    let state = TestHostState::with_secrets(email_secrets(), handler);
    let mut harness = ProviderHarness::new_with_state(spec, state)?;

    // --- ensure ---
    let base_input = json!({ "config": email_config_value() });
    let ensure_input = json!({
        "v": 1,
        "provider": "messaging.email.smtp",
        "resource": "/me/mailFolders('Inbox')/messages",
        "notification_url": "https://example.com/webhook",
        "change_types": ["created"],
        "expiration_minutes": 60,
        "client_state": "client-state",
        "metadata": {"tier": "email"},
        "user": {
            "user_id": "alice",
            "token_key": "msgraph:tenant:alice:refresh_token",
            "tenant_id": "tenant",
            "email": "alice@example.com"
        }
    });
    let mut ensure_payload = base_input.clone();
    ensure_payload
        .as_object_mut()
        .unwrap()
        .extend(ensure_input.as_object().unwrap().clone());
    let response = harness.call(
        "subscription_ensure",
        serde_json::to_vec(&ensure_payload).context("serialize ensure")?,
    )?;
    let result: Value = serde_json::from_slice(&response).context("parse ensure response")?;
    assert!(result["ok"].as_bool().unwrap_or(false));
    assert_eq!(result["subscription"]["subscription_id"], "sub-email");

    // --- renew ---
    let renew_input = json!({
        "v": 1,
        "provider": "messaging.email.smtp",
        "subscription_id": "sub-email",
        "expiration_minutes": 120,
        "user": {
            "user_id": "alice",
            "token_key": "msgraph:tenant:alice:refresh_token",
            "tenant_id": "tenant"
        }
    });
    let mut renew_payload = base_input.clone();
    renew_payload
        .as_object_mut()
        .unwrap()
        .extend(renew_input.as_object().unwrap().clone());
    let response = harness.call(
        "subscription_renew",
        serde_json::to_vec(&renew_payload).context("serialize renew")?,
    )?;
    let result: Value = serde_json::from_slice(&response).context("parse renew")?;
    assert!(result["ok"].as_bool().unwrap_or(false));
    assert_eq!(result["subscription"]["subscription_id"], "sub-email");

    // --- delete ---
    let delete_input = json!({
        "v": 1,
        "provider": "messaging.email.smtp",
        "subscription_id": "sub-email",
        "user": {
            "user_id": "alice",
            "token_key": "msgraph:tenant:alice:refresh_token",
            "tenant_id": "tenant"
        }
    });
    let mut delete_payload = base_input;
    delete_payload
        .as_object_mut()
        .unwrap()
        .extend(delete_input.as_object().unwrap().clone());
    let response = harness.call(
        "subscription_delete",
        serde_json::to_vec(&delete_payload).context("serialize delete")?,
    )?;
    let result: Value = serde_json::from_slice(&response).context("parse delete")?;
    assert!(result["ok"].as_bool().unwrap_or(false));
    assert_eq!(result["subscription"]["subscription_id"], "sub-email");
    Ok(())
}

#[test]
fn email_ingest_graph_notifications() -> Result<()> {
    let spec = provider_spec(ProviderId::Email);
    let handler = move |req: http_client::RequestV1_1| {
        if req.url.contains("/oauth2/v2.0/token") {
            return Ok(graph_token_response("token-ingest"));
        }
        if req.method.eq_ignore_ascii_case("GET") && req.url.contains("/me/messages/") {
            return Ok(graph_json_response(json!({
                "id": "msg-123",
                "subject": "Hello",
                "bodyPreview": "This is a preview",
                "receivedDateTime": "2025-01-01T12:00:00Z",
                "from": {"emailAddress": {"address": "sender@example.com"}},
                "internetMessageId": "<msg-123@example.com>",
                "webLink": "https://graph.microsoft.com/message"
            })));
        }
        Err(http_error("unexpected graph request"))
    };
    let state = TestHostState::with_secrets(email_secrets(), handler);
    let mut harness = ProviderHarness::new_with_state(spec, state)?;
    let config = email_config_value();

    // --- validation token ---
    let validation = HttpInV1 {
        method: "GET".to_string(),
        path: "/webhook".to_string(),
        query: Some("validationToken=abc123".to_string()),
        headers: Vec::new(),
        body_b64: String::new(),
        route_hint: None,
        binding_id: None,
        config: Some(config.clone()),
    };
    let response = harness.call(
        "ingest_http",
        serde_json::to_vec(&validation).context("serialize validation")?,
    )?;
    let http_out: HttpOutV1 =
        serde_json::from_slice(&response).context("parse validation response")?;
    assert_eq!(http_out.status, 200);
    let token = STANDARD
        .decode(&http_out.body_b64)
        .context("decode validation")?;
    assert_eq!(token, b"abc123");

    // --- notification ingestion ---
    let notification = json!({
        "value": [
            {
                "resource": "/me/mailFolders('Inbox')/messages",
                "resourceData": {
                    "id": "msg-123"
                }
            }
        ]
    });
    let ingestion = HttpInV1 {
        method: "POST".to_string(),
        path: "/webhook".to_string(),
        query: None,
        headers: Vec::new(),
        body_b64: STANDARD.encode(&serde_json::to_vec(&notification)?),
        route_hint: None,
        binding_id: Some("alice|msgraph:tenant:alice:refresh_token".to_string()),
        config: Some(config),
    };
    let response = harness.call(
        "ingest_http",
        serde_json::to_vec(&ingestion).context("serialize ingest")?,
    )?;
    let http_out: HttpOutV1 = serde_json::from_slice(&response).context("parse ingest response")?;
    assert_eq!(http_out.status, 200);
    assert_eq!(http_out.events.len(), 1);
    let envelope = &http_out.events[0];
    assert_eq!(
        envelope.metadata.get("graph_message_id").unwrap(),
        "msg-123"
    );
    Ok(())
}
