use chrono::{DateTime, LocalResult, SecondsFormat, TimeZone, Utc};
use greentic_types::messaging::universal_dto::{
    SubscriptionDeleteInV1, SubscriptionDeleteOutV1, SubscriptionEnsureInV1,
    SubscriptionEnsureOutV1, SubscriptionRenewInV1, SubscriptionRenewOutV1,
};
use provider_common::helpers::json_bytes;
use serde_json::{Value, json};
use std::fmt;

use crate::DEFAULT_GRAPH_BASE;
use crate::bindings::greentic::http::http_client as client;
use crate::config::{ProviderConfig, load_config};
use crate::token::acquire_token;

#[derive(Clone, Debug)]
pub(crate) struct ExistingSubscription {
    pub(crate) id: String,
    pub(crate) resource: String,
    pub(crate) change_type: String,
    pub(crate) expiration_datetime: Option<String>,
    pub(crate) notification_url: Option<String>,
}

#[derive(Debug)]
pub(crate) enum GraphRequestError {
    Status(u16),
    Transport(String),
    Parse(String),
}

impl fmt::Display for GraphRequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GraphRequestError::Status(code) => {
                write!(f, "graph request failed with status {}", code)
            }
            GraphRequestError::Transport(err) => write!(f, "{}", err),
            GraphRequestError::Parse(err) => write!(f, "{}", err),
        }
    }
}

impl std::error::Error for GraphRequestError {}

pub(crate) fn subscription_ensure(input_json: &[u8]) -> Vec<u8> {
    match subscription_ensure_inner(input_json) {
        Ok(bytes) => bytes,
        Err(err) => json_bytes(&json!({"ok": false, "error": err})),
    }
}

fn subscription_ensure_inner(input_json: &[u8]) -> Result<Vec<u8>, String> {
    let parsed: Value = serde_json::from_slice(input_json)
        .map_err(|e| format!("invalid json: {e}"))?;

    let dto: SubscriptionEnsureInV1 = serde_json::from_slice(input_json)
        .map_err(|e| format!("invalid subscription ensure input: {e}"))?;

    ensure_provider(&dto.provider)?;

    // Load config from secrets store (don't inject tenant_hint/team_hint
    // into config_value as that triggers partial-config path which misses secrets).
    let cfg = load_config(&parsed)?;

    let token = acquire_token(&cfg)
        .map_err(|e| format!("acquire_token: {e}"))?;

    if dto.change_types.is_empty() {
        return Err("change_types required".into());
    }

    let change_type = dto.change_types.join(",");
    // Default expiration: now + 55 minutes (Teams max is 60 min for channel messages).
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let expiration_target_ms = dto.expiration_target_unix_ms.unwrap_or(now_ms + 55 * 60 * 1000);
    let expiration_iso = expiration_ms_to_iso(expiration_target_ms)?;

    let client_state = dto.client_state.clone().or_else(|| dto.binding_id.clone());

    let subscription = match create_subscription(
        &cfg,
        &token,
        &dto.notification_url,
        &dto.resource,
        &change_type,
        &expiration_iso,
        client_state.as_deref(),
    ) {
        Ok(sub) => sub,
        Err(err) => {
            if matches!(err, GraphRequestError::Status(409)) {
                let existing = list_subscriptions(&cfg, &token)
                    .map_err(|e| format!("list_subscriptions: {e}"))?;
                if let Some(found) = existing.into_iter().find(|sub| {
                    sub.resource == dto.resource
                        && sub.change_type == change_type
                        && sub
                            .notification_url
                            .as_deref()
                            .map(|url| url == dto.notification_url)
                            .unwrap_or(false)
                }) {
                    renew_subscription(&cfg, &token, &found.id, &expiration_iso)
                        .map_err(|e| format!("renew_subscription: {e}"))?;
                    let mut updated = found.clone();
                    updated.expiration_datetime = Some(expiration_iso.clone());
                    updated
                } else {
                    return Err("subscription conflict: existing subscription not found".into());
                }
            } else {
                return Err(format!("create_subscription: {err}"));
            }
        }
    };

    let expiration_unix_ms = match subscription.expiration_datetime.as_deref() {
        Some(datetime) => parse_expiration_ms(datetime).unwrap_or(expiration_target_ms),
        None => expiration_target_ms,
    };

    let out = SubscriptionEnsureOutV1 {
        v: 1,
        subscription_id: subscription.id.clone(),
        expiration_unix_ms,
        resource: subscription.resource.clone(),
        change_types: dto.change_types.clone(),
        client_state,
        metadata: dto.metadata.clone(),
        binding_id: dto.binding_id.clone(),
        user: dto.user.clone(),
    };
    Ok(json_bytes(&json!({"ok": true, "subscription": out})))
}

pub(crate) fn subscription_renew(input_json: &[u8]) -> Vec<u8> {
    let parsed: Value = match serde_json::from_slice(input_json) {
        Ok(value) => value,
        Err(err) => {
            return json_bytes(&json!({"ok": false, "error": format!("invalid json: {err}")}));
        }
    };

    let dto = match serde_json::from_slice::<SubscriptionRenewInV1>(input_json) {
        Ok(value) => value,
        Err(err) => {
            return json_bytes(
                &json!({"ok": false, "error": format!("invalid subscription renew input: {err}")}),
            );
        }
    };

    if let Err(err) = ensure_provider(&dto.provider) {
        return json_bytes(&json!({"ok": false, "error": err}));
    }

    let cfg = match load_config(&parsed) {
        Ok(cfg) => cfg,
        Err(err) => return json_bytes(&json!({"ok": false, "error": err})),
    };

    let token = match acquire_token(&cfg) {
        Ok(token) => token,
        Err(err) => return json_bytes(&json!({"ok": false, "error": err})),
    };

    let expiration_target_ms = match dto.expiration_target_unix_ms {
        Some(ms) => ms,
        None => {
            return json_bytes(
                &json!({"ok": false, "error": "expiration_target_unix_ms required"}),
            );
        }
    };
    let expiration_iso = match expiration_ms_to_iso(expiration_target_ms) {
        Ok(text) => text,
        Err(err) => return json_bytes(&json!({"ok": false, "error": err})),
    };

    if let Err(err) = renew_subscription(&cfg, &token, &dto.subscription_id, &expiration_iso) {
        return json_bytes(&json!({"ok": false, "error": err.to_string()}));
    }

    let expiration_unix_ms = parse_expiration_ms(&expiration_iso).unwrap_or(expiration_target_ms);
    let out = SubscriptionRenewOutV1 {
        v: 1,
        subscription_id: dto.subscription_id,
        expiration_unix_ms,
        metadata: dto.metadata,
        user: dto.user.clone(),
    };
    json_bytes(&json!({"ok": true, "subscription": out}))
}

pub(crate) fn subscription_delete(input_json: &[u8]) -> Vec<u8> {
    let parsed: Value = match serde_json::from_slice(input_json) {
        Ok(value) => value,
        Err(err) => {
            return json_bytes(&json!({"ok": false, "error": format!("invalid json: {err}")}));
        }
    };

    let dto = match serde_json::from_slice::<SubscriptionDeleteInV1>(input_json) {
        Ok(value) => value,
        Err(err) => {
            return json_bytes(
                &json!({"ok": false, "error": format!("invalid subscription delete input: {err}")}),
            );
        }
    };

    if let Err(err) = ensure_provider(&dto.provider) {
        return json_bytes(&json!({"ok": false, "error": err}));
    }

    let cfg = match load_config(&parsed) {
        Ok(cfg) => cfg,
        Err(err) => return json_bytes(&json!({"ok": false, "error": err})),
    };

    let token = match acquire_token(&cfg) {
        Ok(token) => token,
        Err(err) => return json_bytes(&json!({"ok": false, "error": err})),
    };

    if let Err(err) = delete_subscription(&cfg, &token, &dto.subscription_id) {
        return json_bytes(&json!({"ok": false, "error": err.to_string()}));
    }

    let out = SubscriptionDeleteOutV1 {
        v: 1,
        subscription_id: dto.subscription_id,
        user: dto.user.clone(),
    };
    json_bytes(&json!({"ok": true, "subscription": out}))
}

pub(crate) fn ensure_provider(provider: &str) -> Result<(), String> {
    match provider {
        "teams" | "msgraph" | "messaging-teams" => Ok(()),
        other => Err(format!("unsupported provider: {other}")),
    }
}

pub(crate) fn expiration_ms_to_iso(ms: u64) -> Result<String, String> {
    let secs = (ms / 1000) as i64;
    let nanos = ((ms % 1000) * 1_000_000) as u32;
    match Utc.timestamp_opt(secs, nanos) {
        LocalResult::Single(datetime) => Ok(datetime.to_rfc3339_opts(SecondsFormat::Secs, true)),
        _ => Err("invalid expiration timestamp".to_string()),
    }
}

pub(crate) fn parse_expiration_ms(value: &str) -> Result<u64, String> {
    let dt = DateTime::parse_from_rfc3339(value)
        .map_err(|e| format!("invalid expiration datetime: {e}"))?;
    Ok(dt.timestamp_millis() as u64)
}

pub(crate) fn list_subscriptions(
    cfg: &ProviderConfig,
    token: &str,
) -> Result<Vec<ExistingSubscription>, GraphRequestError> {
    let graph_base = cfg
        .graph_base_url
        .clone()
        .unwrap_or_else(|| DEFAULT_GRAPH_BASE.to_string());
    let url = format!("{}/subscriptions", graph_base);
    let request = client::Request {
        method: "GET".into(),
        url,
        headers: vec![("Authorization".into(), format!("Bearer {}", token))],
        body: None,
    };
    let resp = client::send(&request, None, None)
        .map_err(|e| GraphRequestError::Transport(format!("transport error: {}", e.message)))?;
    if resp.status < 200 || resp.status >= 300 {
        return Err(GraphRequestError::Status(resp.status));
    }
    let body = resp.body.unwrap_or_default();
    let json: Value = serde_json::from_slice(&body)
        .map_err(|e| GraphRequestError::Parse(format!("invalid subscriptions response: {e}")))?;
    let list = json
        .get("value")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    for item in list {
        let id = item.get("id").and_then(Value::as_str);
        let resource = item.get("resource").and_then(Value::as_str);
        let change_type = item.get("changeType").and_then(Value::as_str);
        if let (Some(id), Some(resource), Some(change_type)) = (id, resource, change_type) {
            out.push(ExistingSubscription {
                id: id.to_string(),
                resource: resource.to_string(),
                change_type: change_type.to_string(),
                expiration_datetime: item
                    .get("expirationDateTime")
                    .and_then(Value::as_str)
                    .map(|s| s.to_string()),
                notification_url: item
                    .get("notificationUrl")
                    .and_then(Value::as_str)
                    .map(|s| s.to_string()),
            });
        }
    }
    Ok(out)
}

pub(crate) fn create_subscription(
    cfg: &ProviderConfig,
    token: &str,
    notification_url: &str,
    resource: &str,
    change_type: &str,
    expiration: &str,
    client_state: Option<&str>,
) -> Result<ExistingSubscription, GraphRequestError> {
    let graph_base = cfg
        .graph_base_url
        .clone()
        .unwrap_or_else(|| DEFAULT_GRAPH_BASE.to_string());
    let url = format!("{}/subscriptions", graph_base);
    let mut payload = json!({
        "changeType": change_type,
        "notificationUrl": notification_url,
        "resource": resource,
        "expirationDateTime": expiration,
    });
    if let Some(state) = client_state {
        payload
            .as_object_mut()
            .expect("payload object")
            .insert("clientState".into(), Value::String(state.to_string()));
    }
    let request = client::Request {
        method: "POST".into(),
        url,
        headers: vec![
            ("Content-Type".into(), "application/json".into()),
            ("Authorization".into(), format!("Bearer {}", token)),
        ],
        body: Some(serde_json::to_vec(&payload).unwrap_or_else(|_| b"{}".to_vec())),
    };
    let resp = client::send(&request, None, None)
        .map_err(|e| GraphRequestError::Transport(format!("transport error: {}", e.message)))?;
    if resp.status < 200 || resp.status >= 300 {
        let err_body = resp
            .body
            .as_ref()
            .and_then(|b| String::from_utf8(b.clone()).ok())
            .unwrap_or_default();
        eprintln!(
            "teams create_subscription failed: status={} body={}",
            resp.status, err_body
        );
        return Err(GraphRequestError::Transport(format!(
            "create subscription failed: status={} body={}",
            resp.status, err_body
        )));
    }
    let body = resp.body.unwrap_or_default();
    let json: Value = serde_json::from_slice(&body)
        .map_err(|e| GraphRequestError::Parse(format!("invalid create response: {e}")))?;
    let id = json
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| GraphRequestError::Parse("create response missing id".to_string()))?;
    Ok(ExistingSubscription {
        id: id.to_string(),
        resource: resource.to_string(),
        change_type: change_type.to_string(),
        expiration_datetime: json
            .get("expirationDateTime")
            .and_then(Value::as_str)
            .map(|s| s.to_string())
            .or_else(|| Some(expiration.to_string())),
        notification_url: Some(notification_url.to_string()),
    })
}

pub(crate) fn renew_subscription(
    cfg: &ProviderConfig,
    token: &str,
    subscription_id: &str,
    expiration: &str,
) -> Result<(), GraphRequestError> {
    let graph_base = cfg
        .graph_base_url
        .clone()
        .unwrap_or_else(|| DEFAULT_GRAPH_BASE.to_string());
    let url = format!("{}/subscriptions/{}", graph_base, subscription_id);
    let payload = json!({ "expirationDateTime": expiration });
    let request = client::Request {
        method: "PATCH".into(),
        url,
        headers: vec![
            ("Content-Type".into(), "application/json".into()),
            ("Authorization".into(), format!("Bearer {}", token)),
        ],
        body: Some(serde_json::to_vec(&payload).unwrap_or_else(|_| b"{}".to_vec())),
    };
    let resp = client::send(&request, None, None)
        .map_err(|e| GraphRequestError::Transport(format!("transport error: {}", e.message)))?;
    if resp.status < 200 || resp.status >= 300 {
        return Err(GraphRequestError::Status(resp.status));
    }
    Ok(())
}

pub(crate) fn delete_subscription(
    cfg: &ProviderConfig,
    token: &str,
    subscription_id: &str,
) -> Result<(), GraphRequestError> {
    let graph_base = cfg
        .graph_base_url
        .clone()
        .unwrap_or_else(|| DEFAULT_GRAPH_BASE.to_string());
    let url = format!("{}/subscriptions/{}", graph_base, subscription_id);
    let request = client::Request {
        method: "DELETE".into(),
        url,
        headers: vec![("Authorization".into(), format!("Bearer {}", token))],
        body: None,
    };
    let resp = client::send(&request, None, None)
        .map_err(|e| GraphRequestError::Transport(format!("transport error: {}", e.message)))?;
    if resp.status < 200 || resp.status >= 300 {
        return Err(GraphRequestError::Status(resp.status));
    }
    Ok(())
}
