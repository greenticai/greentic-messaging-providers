use base64::{Engine, engine::general_purpose::STANDARD};
use greentic_types::messaging::universal_dto::{AuthUserRefV1, Header, HttpInV1, HttpOutV1};
use provider_common::http_compat::{
    http_out_error, http_out_v1_bytes, parse_operator_http_in_with_config,
};
use serde_json::Value;
use urlencoding::decode as url_decode;

use crate::auth;
use crate::config::{ProviderConfig, parse_config_value};
use crate::graph::{graph_base_url, graph_get};
use greentic_types::{
    Actor, ChannelMessageEnvelope, Destination, EnvId, MessageMetadata, TenantCtx, TenantId,
};

pub(crate) fn ingest_http(input_json: &[u8]) -> Vec<u8> {
    // Try native greentic-types format first, fall back to operator format
    let http = match serde_json::from_slice::<HttpInV1>(input_json) {
        Ok(value) => value,
        Err(_) => match parse_operator_http_in_with_config(input_json) {
            Ok(req) => req,
            Err(err) => return http_out_error(400, &format!("invalid http input: {err}")),
        },
    };
    match http.method.to_uppercase().as_str() {
        "GET" => handle_validation(&http),
        "POST" => handle_graph_notifications(&http),
        _ => http_out_error(405, "method not allowed"),
    }
}

pub(crate) fn handle_validation(http: &HttpInV1) -> Vec<u8> {
    let token = http
        .query
        .as_deref()
        .and_then(|query| query_param_value(query, "validationToken"))
        .unwrap_or_default();
    if token.is_empty() {
        return http_out_error(400, "validationToken missing");
    }
    let headers = vec![Header {
        name: "Content-Type".into(),
        value: "text/plain".into(),
    }];
    let out = HttpOutV1 {
        status: 200,
        headers,
        body_b64: STANDARD.encode(token.as_bytes()),
        events: Vec::new(),
    };
    http_out_v1_bytes(&out)
}

fn handle_graph_notifications(http: &HttpInV1) -> Vec<u8> {
    let config_value = match http.config.as_ref() {
        Some(cfg) => cfg,
        None => return http_out_error(400, "config required for ingest"),
    };
    let cfg = match parse_config_value(config_value) {
        Ok(cfg) => cfg,
        Err(err) => return http_out_error(400, &err),
    };
    let user = match binding_to_user(http.binding_id.as_ref()) {
        Ok(value) => value,
        Err(err) => return http_out_error(400, &err),
    };
    let token = match auth::acquire_graph_token(&cfg, &user) {
        Ok(value) => value,
        Err(err) => return http_out_error(500, &err),
    };
    let notifications = match parse_graph_notifications(&http.body_b64) {
        Ok(value) => value,
        Err(err) => return http_out_error(400, &err),
    };
    let mut events = Vec::new();
    for (resource, message_id) in notifications {
        match fetch_graph_message(&token, &cfg, &message_id) {
            Ok(message) => {
                events.push(channel_message_envelope(
                    &message,
                    &user,
                    &message_id,
                    &resource,
                ));
            }
            Err(err) => return http_out_error(500, &err),
        }
    }
    let out = HttpOutV1 {
        status: 200,
        headers: Vec::new(),
        body_b64: String::new(),
        events,
    };
    http_out_v1_bytes(&out)
}

fn query_param_value(query: &str, key: &str) -> Option<String> {
    for part in query.split('&') {
        let mut kv = part.splitn(2, '=');
        if let Some(k) = kv.next()
            && k == key
            && let Some(v) = kv.next()
        {
            return url_decode(v).ok().map(|cow| cow.into_owned());
        }
    }
    None
}

pub(crate) fn binding_to_user(binding: Option<&String>) -> Result<AuthUserRefV1, String> {
    let binding = binding.ok_or_else(|| "binding_id required".to_string())?;
    let parts: Vec<&str> = binding.splitn(2, '|').collect();
    let (user_id, token_key) = if parts.len() == 2 {
        (parts[0], parts[1])
    } else {
        (binding.as_str(), binding.as_str())
    };
    Ok(AuthUserRefV1 {
        user_id: user_id.to_string(),
        token_key: token_key.to_string(),
        tenant_id: None,
        email: None,
        display_name: None,
    })
}

fn parse_graph_notifications(body_b64: &str) -> Result<Vec<(String, String)>, String> {
    let bytes = STANDARD
        .decode(body_b64)
        .map_err(|err| format!("invalid notification body: {err}"))?;
    let json: Value = serde_json::from_slice(&bytes)
        .map_err(|err| format!("notification decode failed: {err}"))?;
    let entries = json
        .get("value")
        .and_then(Value::as_array)
        .ok_or_else(|| "missing notification value array".to_string())?;
    let mut parsed = Vec::new();
    for entry in entries {
        let resource = entry
            .get("resource")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let message_id = entry
            .get("resourceData")
            .and_then(|rd| rd.get("id"))
            .and_then(Value::as_str)
            .or_else(|| {
                entry
                    .get("resourceData")
                    .and_then(|rd| rd.get("@odata.id"))
                    .and_then(Value::as_str)
            })
            .ok_or_else(|| "notification missing resourceData.id".to_string())?
            .to_string();
        parsed.push((resource, message_id));
    }
    Ok(parsed)
}

fn fetch_graph_message(
    token: &str,
    cfg: &ProviderConfig,
    message_id: &str,
) -> Result<Value, String> {
    let base = graph_base_url(cfg);
    let url = format!(
        "{}/me/messages/{}?$select=subject,bodyPreview,receivedDateTime,from,toRecipients,webLink,internetMessageId",
        base, message_id
    );
    graph_get(token, &url)
}

fn channel_message_envelope(
    message: &Value,
    user: &AuthUserRefV1,
    message_id: &str,
    resource: &str,
) -> ChannelMessageEnvelope {
    let subject = message
        .get("subject")
        .and_then(Value::as_str)
        .unwrap_or("email message")
        .to_string();
    let preview = message
        .get("bodyPreview")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let received = message
        .get("receivedDateTime")
        .and_then(Value::as_str)
        .unwrap_or("");
    let from_address = message
        .get("from")
        .and_then(|from| from.get("emailAddress"))
        .and_then(|ea| ea.get("address"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let mut metadata = MessageMetadata::new();
    metadata.insert("graph_message_id".to_string(), message_id.to_string());
    metadata.insert("subject".to_string(), subject.clone());
    if !preview.is_empty() {
        metadata.insert("body_preview".to_string(), preview);
    }
    if !received.is_empty() {
        metadata.insert("receivedDateTime".to_string(), received.to_string());
    }
    if !from_address.is_empty() {
        metadata.insert("from".to_string(), from_address.to_string());
        metadata.insert("to".to_string(), from_address.to_string());
    }
    metadata.insert("resource".to_string(), resource.to_string());
    let env = default_env();
    let tenant = default_tenant();
    let destinations = if !from_address.is_empty() {
        vec![Destination {
            id: from_address.to_string(),
            kind: Some("email".into()),
        }]
    } else {
        Vec::new()
    };
    ChannelMessageEnvelope {
        id: format!("email-{message_id}"),
        tenant: TenantCtx::new(env, tenant),
        channel: "email".to_string(),
        session_id: message_id.to_string(),
        reply_scope: None,
        from: Some(Actor {
            id: user.user_id.clone(),
            kind: Some("user".into()),
        }),
        to: destinations,
        correlation_id: Some(resource.to_string()),
        text: Some(subject),
        attachments: Vec::new(),
        metadata,
        extensions: Default::default(),
    }
}

fn default_env() -> EnvId {
    EnvId::try_from("default").expect("default env id present")
}

fn default_tenant() -> TenantId {
    TenantId::try_from("default").expect("default tenant id present")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn http(method: &str, query: Option<&str>, body_b64: String) -> HttpInV1 {
        HttpInV1 {
            method: method.to_string(),
            path: "/webhook".to_string(),
            query: query.map(str::to_string),
            headers: Vec::new(),
            body_b64,
            config: None,
            binding_id: None,
            route_hint: None,
        }
    }

    #[test]
    fn validation_echoes_url_decoded_token_as_plain_text() {
        let out = handle_validation(&http(
            "GET",
            Some("validationToken=hello%20graph"),
            String::new(),
        ));
        let parsed: HttpOutV1 = serde_json::from_slice(&out).expect("http out");
        let body = STANDARD.decode(parsed.body_b64).expect("body");

        assert_eq!(parsed.status, 200);
        assert_eq!(String::from_utf8(body).unwrap(), "hello graph");
        assert_eq!(parsed.headers[0].name, "Content-Type");
        assert_eq!(parsed.headers[0].value, "text/plain");
    }

    #[test]
    fn validation_rejects_missing_token() {
        let out = handle_validation(&http("GET", Some("x=1"), String::new()));
        let parsed: HttpOutV1 = serde_json::from_slice(&out).expect("http out");

        assert_eq!(parsed.status, 400);
    }

    #[test]
    fn binding_to_user_splits_user_id_and_token_key() {
        let binding = "user@example.com|refresh-token-key".to_string();
        let user = binding_to_user(Some(&binding)).expect("user");

        assert_eq!(user.user_id, "user@example.com");
        assert_eq!(user.token_key, "refresh-token-key");

        let simple = "same-key".to_string();
        let user = binding_to_user(Some(&simple)).expect("user");
        assert_eq!(user.user_id, "same-key");
        assert_eq!(user.token_key, "same-key");
    }

    #[test]
    fn binding_to_user_requires_binding_id() {
        let err = binding_to_user(None).unwrap_err();

        assert_eq!(err, "binding_id required");
    }

    #[test]
    fn notification_parser_accepts_id_and_odata_id() {
        let payload = json!({
            "value": [
                {"resource": "me/messages/a", "resourceData": {"id": "a"}},
                {"resource": "me/messages/b", "resourceData": {"@odata.id": "b"}}
            ]
        });
        let encoded = STANDARD.encode(payload.to_string());

        let parsed = parse_graph_notifications(&encoded).expect("notifications");

        assert_eq!(parsed[0], ("me/messages/a".to_string(), "a".to_string()));
        assert_eq!(parsed[1], ("me/messages/b".to_string(), "b".to_string()));
    }

    #[test]
    fn notification_parser_reports_missing_message_id() {
        let encoded = STANDARD.encode(r#"{"value":[{"resourceData":{}}]}"#);

        let err = parse_graph_notifications(&encoded).unwrap_err();

        assert_eq!(err, "notification missing resourceData.id");
    }

    #[test]
    fn channel_message_envelope_maps_graph_message_fields() {
        let user = AuthUserRefV1 {
            user_id: "u1".to_string(),
            token_key: "token".to_string(),
            tenant_id: None,
            email: None,
            display_name: None,
        };
        let message = json!({
            "subject": "Hello",
            "bodyPreview": "Preview",
            "receivedDateTime": "2026-01-02T03:04:05Z",
            "from": {"emailAddress": {"address": "sender@example.com"}}
        });

        let env = channel_message_envelope(&message, &user, "msg-1", "me/messages/msg-1");

        assert_eq!(env.id, "email-msg-1");
        assert_eq!(env.text.as_deref(), Some("Hello"));
        assert_eq!(env.to[0].id, "sender@example.com");
        assert_eq!(
            env.metadata.get("body_preview").map(String::as_str),
            Some("Preview")
        );
        assert_eq!(
            env.metadata.get("resource").map(String::as_str),
            Some("me/messages/msg-1")
        );
    }

    #[test]
    fn email_deliberately_emits_no_attachments_in_v1() {
        // Graph delegated tokens are not available to the host; deferred by decision
        // (spec section 6.1). Remove this test only together with real support.
        let user = AuthUserRefV1 {
            user_id: "u1".to_string(),
            token_key: "token".to_string(),
            tenant_id: None,
            email: None,
            display_name: None,
        };
        let message = json!({
            "subject": "Invoice",
            "bodyPreview": "see attached",
            "receivedDateTime": "2026-01-02T03:04:05Z",
            "from": {"emailAddress": {"address": "sender@example.com"}},
            "hasAttachments": true,
            "attachments": [{"name": "a.pdf", "contentType": "application/pdf", "size": 1234}]
        });
        let env = channel_message_envelope(&message, &user, "msg-2", "me/messages/msg-2");
        assert!(env.attachments.is_empty());
        assert!(!env.extensions.contains_key("attachment_fetch"));
        assert!(!env.extensions.contains_key("artifacts"));
    }
}
