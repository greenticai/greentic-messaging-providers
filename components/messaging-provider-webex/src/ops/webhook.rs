//! `setup_webhook` operation for Webex event delivery.
//!
//! This registers Webex -> Greentic callbacks with the Webex Webhooks API.

use provider_common::helpers::json_bytes;
use serde_json::{Value, json};

use crate::DEFAULT_API_BASE;
use crate::bindings::greentic::http::http_client as client;
#[cfg(not(test))]
use crate::config::get_secret_string;
use crate::config::load_config;
#[cfg(not(test))]
use crate::{DEFAULT_TOKEN_KEY, DEFAULT_WEBHOOK_SECRET_KEY};

type HttpSend<'a> = dyn FnMut(&client::Request) -> Result<client::Response, client::HostError> + 'a;

/// Component-side operational log. The runner host forwards WASM stderr to
/// the operator console, so these lines surface next to the host's own
/// `[setup-action ...]` logs. Never log token/secret VALUES here —
/// presence/absence only.
fn wlog(msg: &str) {
    eprintln!("[messaging-webex] {msg}");
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WebhookSpec {
    name: String,
    target_url: String,
    resource: &'static str,
    event: &'static str,
    filter: Option<String>,
    secret: Option<String>,
}

struct WebhookSetup<'a> {
    api_base: &'a str,
    token: &'a str,
    target_url: &'a str,
    instance: &'a str,
    room_id: Option<&'a str>,
    secret: Option<&'a str>,
}

/// Handle `setup_webhook` op.
pub(crate) fn setup_webhook(input_json: &[u8]) -> Vec<u8> {
    let parsed: Value = match serde_json::from_slice(input_json) {
        Ok(val) => val,
        Err(err) => {
            return json_bytes(&json!({"ok": false, "error": format!("invalid json: {err}")}));
        }
    };

    let cfg = load_config(&parsed).ok();
    let token = match resolve_token(&parsed, cfg.as_ref()) {
        Ok(token) => token,
        Err(err) => {
            wlog(&format!("setup_webhook: bot token unavailable: {err}"));
            return json_bytes(&json!({"ok": false, "error": err}));
        }
    };
    let api_base = resolve_api_base_url(&parsed, cfg.as_ref());
    let tenant = input_string(&parsed, "tenant").unwrap_or_else(|| "default".to_string());
    let channel = input_string(&parsed, "channel")
        .or_else(|| input_string(&parsed, "team"))
        .or_else(|| input_string(&parsed, "provider_instance_id"))
        .unwrap_or_else(|| "default".to_string());
    let instance = sanitize_name_part(&format!("{tenant}-{channel}"));

    // Revision-serve passes a fully-formed `webhook_url`; legacy derives the
    // Webex target URL from `public_base_url` + tenant/channel.
    let target_url = match input_string(&parsed, "webhook_url") {
        Some(url) => {
            if !url.starts_with("https://") {
                return json_bytes(
                    &json!({"ok": false, "error": "webhook_url must be an https:// URL"}),
                );
            }
            url
        }
        None => {
            let public_base_url = match resolve_public_base_url(&parsed, cfg.as_ref()) {
                Ok(url) => url,
                Err(err) => return json_bytes(&json!({"ok": false, "error": err})),
            };
            build_target_url(&public_base_url, &tenant, &channel)
        }
    };
    let room_id = input_string(&parsed, "room_id")
        .or_else(|| input_string(&parsed, "webex_room_id"))
        .or_else(|| cfg.as_ref().and_then(|c| c.default_room_id.clone()));
    let secret = input_string(&parsed, "webhook_secret")
        .or_else(|| cfg.as_ref().and_then(|c| c.webhook_secret.clone()))
        .or_else(resolve_webhook_secret);
    wlog(&format!(
        "setup_webhook: reconciling webhooks for instance {instance} → {target_url} \
         (room_id={}, webhook_secret={})",
        if room_id.is_some() {
            "present"
        } else {
            "absent"
        },
        if secret.is_some() {
            "present"
        } else {
            "absent"
        },
    ));
    let mut send = |request: &client::Request| client::send(request, None, None);
    let result = setup_webhook_with_sender(
        WebhookSetup {
            api_base: &api_base,
            token: &token,
            target_url: &target_url,
            instance: &instance,
            room_id: room_id.as_deref(),
            secret: secret.as_deref(),
        },
        &parsed,
        &mut send,
    );
    json_bytes(&result)
}

fn setup_webhook_with_sender(
    setup: WebhookSetup<'_>,
    parsed: &Value,
    send: &mut HttpSend<'_>,
) -> Value {
    let bot_person_id = if setup.room_id.is_none() {
        match input_string(parsed, "bot_person_id")
            .or_else(|| input_string(parsed, "webex_bot_person_id"))
            .map(Ok)
            .unwrap_or_else(|| fetch_bot_person_id(setup.api_base, setup.token, send))
        {
            Ok(id) => Some(id),
            Err(err) => {
                wlog(&format!(
                    "setup_webhook: GET /people/me (bot identity) failed: {err}"
                ));
                return json!({"ok": false, "error": err});
            }
        }
    } else {
        None
    };

    let specs = webhook_specs(
        setup.instance,
        setup.target_url,
        setup.room_id,
        bot_person_id.as_deref(),
        setup.secret,
    );
    let existing = match list_webhooks(setup.api_base, setup.token, send) {
        Ok(list) => list,
        Err(err) => {
            wlog(&format!("setup_webhook: GET /webhooks failed: {err}"));
            return json!({
                "ok": false,
                "error": format!("list webex webhooks failed: {err}")
            });
        }
    };

    let mut all_ok = true;
    let mut results = Vec::new();
    let stale = stale_webhooks_for_instance(&existing, &specs, setup.instance);
    wlog(&format!(
        "setup_webhook: {} existing webhook(s) listed, {} spec(s) desired, {} stale to delete",
        existing.len(),
        specs.len(),
        stale.len()
    ));
    for wh in &stale {
        wlog(&format!(
            "setup_webhook: deleting {} ({}.{}) — it no longer matches the desired target URL or signing secret",
            value_str(wh, "name"),
            value_str(wh, "resource"),
            value_str(wh, "event"),
        ));
        let result = delete_webhook(setup.api_base, setup.token, wh, send);
        if result.get("ok").and_then(Value::as_bool) != Some(true) {
            all_ok = false;
        }
        results.push(result);
    }

    let stale_ids: std::collections::HashSet<String> = stale
        .iter()
        .filter_map(|wh| wh.get("id").and_then(Value::as_str).map(ToOwned::to_owned))
        .collect();
    let active_existing: Vec<Value> = existing
        .iter()
        .filter(|wh| {
            wh.get("id")
                .and_then(Value::as_str)
                .is_none_or(|id| !stale_ids.contains(id))
        })
        .cloned()
        .collect();

    for spec in specs {
        let result = reconcile_webhook(setup.api_base, setup.token, &active_existing, &spec, send);
        if result.get("ok").and_then(Value::as_bool) != Some(true) {
            all_ok = false;
            wlog(&format!(
                "setup_webhook: reconcile {}.{} failed: {}",
                result
                    .get("resource")
                    .and_then(Value::as_str)
                    .unwrap_or("?"),
                result.get("event").and_then(Value::as_str).unwrap_or("?"),
                result
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("see webex_response"),
            ));
        }
        results.push(result);
    }

    if all_ok {
        wlog(&format!(
            "setup_webhook: done — all webhooks reconciled to {}",
            setup.target_url
        ));
    } else {
        wlog("setup_webhook: done with FAILURES — see per-webhook results above");
    }
    json!({
        "ok": all_ok,
        "target_url": setup.target_url,
        "webhook_url": setup.target_url,
        "webhooks": results,
    })
}

fn resolve_token(parsed: &Value, cfg: Option<&crate::ProviderConfig>) -> Result<String, String> {
    input_string(parsed, "bot_token")
        .or_else(|| input_string(parsed, "webex_bot_token"))
        .or_else(|| cfg.and_then(|c| c.bot_token.clone()))
        .or_else(resolve_token_secret)
        .ok_or_else(|| {
            "missing Webex bot token (bot_token, webex_bot_token, or WEBEX_BOT_TOKEN secret)"
                .to_string()
        })
}

#[cfg(not(test))]
fn resolve_token_secret() -> Option<String> {
    get_secret_string(DEFAULT_TOKEN_KEY).ok()
}

#[cfg(test)]
fn resolve_token_secret() -> Option<String> {
    None
}

#[cfg(not(test))]
fn resolve_webhook_secret() -> Option<String> {
    get_secret_string(DEFAULT_WEBHOOK_SECRET_KEY).ok()
}

#[cfg(test)]
fn resolve_webhook_secret() -> Option<String> {
    None
}

fn resolve_public_base_url(
    parsed: &Value,
    cfg: Option<&crate::ProviderConfig>,
) -> Result<String, String> {
    let public_base_url = input_string(parsed, "public_base_url")
        .or_else(|| cfg.map(|c| c.public_base_url.clone()))
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty() && value != "https://invalid.local")
        .ok_or_else(|| "missing public_base_url for Webex webhook setup".to_string())?;
    if !public_base_url.starts_with("https://") {
        return Err("public_base_url must be an https:// URL for Webex webhooks".to_string());
    }
    Ok(public_base_url)
}

fn resolve_api_base_url(parsed: &Value, cfg: Option<&crate::ProviderConfig>) -> String {
    input_string(parsed, "api_base_url")
        .or_else(|| cfg.and_then(|c| c.api_base_url.clone()))
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_API_BASE.to_string())
}

fn input_string(parsed: &Value, key: &str) -> Option<String> {
    parsed
        .get(key)
        .or_else(|| parsed.get("config").and_then(|cfg| cfg.get(key)))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

fn build_target_url(public_base_url: &str, tenant: &str, channel: &str) -> String {
    format!(
        "{}/v1/messaging/ingress/messaging-webex/{}/{}",
        public_base_url.trim_end_matches('/'),
        url_segment(tenant),
        url_segment(channel),
    )
}

fn webhook_specs(
    instance: &str,
    target_url: &str,
    room_id: Option<&str>,
    bot_person_id: Option<&str>,
    secret: Option<&str>,
) -> Vec<WebhookSpec> {
    let message_specs = room_id
        .map(|room| {
            vec![WebhookSpec {
                name: format!("greentic-webex-{instance}-messages-created"),
                target_url: target_url.to_string(),
                resource: "messages",
                event: "created",
                filter: Some(format!("roomId={room}")),
                secret: secret.map(ToOwned::to_owned),
            }]
        })
        .unwrap_or_else(|| {
            vec![
                WebhookSpec {
                    name: format!("greentic-webex-{instance}-messages-mentioned-created"),
                    target_url: target_url.to_string(),
                    resource: "messages",
                    event: "created",
                    filter: bot_person_id.map(|id| format!("mentionedPeople={id}")),
                    secret: secret.map(ToOwned::to_owned),
                },
                WebhookSpec {
                    name: format!("greentic-webex-{instance}-messages-direct-created"),
                    target_url: target_url.to_string(),
                    resource: "messages",
                    event: "created",
                    filter: Some("roomType=direct".to_string()),
                    secret: secret.map(ToOwned::to_owned),
                },
            ]
        });
    let actions_filter = room_id.map(|room| format!("roomId={room}"));
    message_specs
        .into_iter()
        .chain(std::iter::once(WebhookSpec {
            name: format!("greentic-webex-{instance}-attachment-actions-created"),
            target_url: target_url.to_string(),
            resource: "attachmentActions",
            event: "created",
            filter: actions_filter,
            secret: secret.map(ToOwned::to_owned),
        }))
        .collect()
}

fn fetch_bot_person_id(
    api_base: &str,
    token: &str,
    send: &mut HttpSend<'_>,
) -> Result<String, String> {
    let resp = send(&client::Request {
        method: "GET".to_string(),
        url: format!("{api_base}/people/me"),
        headers: vec![("Authorization".to_string(), format!("Bearer {token}"))],
        body: None,
    })
    .map_err(|e| format!("http error: {}", e.message))?;
    let body_bytes = resp.body.unwrap_or_default();
    let body: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    if !(200..300).contains(&(resp.status as u32)) {
        return Err(format!(
            "status {} body={}",
            resp.status,
            String::from_utf8_lossy(&body_bytes)
        ));
    }
    body.get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| "people/me response did not include bot person id".to_string())
}

fn list_webhooks(
    api_base: &str,
    token: &str,
    send: &mut HttpSend<'_>,
) -> Result<Vec<Value>, String> {
    let resp = send(&client::Request {
        method: "GET".to_string(),
        url: format!("{api_base}/webhooks"),
        headers: vec![("Authorization".to_string(), format!("Bearer {token}"))],
        body: None,
    })
    .map_err(|e| format!("http error: {}", e.message))?;
    let body_bytes = resp.body.unwrap_or_default();
    let body: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    if !(200..300).contains(&(resp.status as u32)) {
        return Err(format!(
            "status {} body={}",
            resp.status,
            String::from_utf8_lossy(&body_bytes)
        ));
    }
    Ok(body
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

fn stale_webhooks_for_instance(
    existing: &[Value],
    specs: &[WebhookSpec],
    instance: &str,
) -> Vec<Value> {
    let keep_indexes = current_webhook_indexes(existing, specs);
    existing
        .iter()
        .enumerate()
        .filter(|(idx, wh)| {
            is_greentic_webex_webhook_for_instance(wh, instance) && !keep_indexes.contains(idx)
        })
        .map(|(_, wh)| wh.clone())
        .collect()
}

fn current_webhook_indexes(
    existing: &[Value],
    specs: &[WebhookSpec],
) -> std::collections::HashSet<usize> {
    let mut keep = std::collections::HashSet::new();
    for spec in specs {
        if let Some((idx, _)) = existing
            .iter()
            .enumerate()
            .find(|(idx, wh)| !keep.contains(idx) && existing_matches(wh, spec))
        {
            keep.insert(idx);
        }
    }
    keep
}

fn is_greentic_webex_webhook_for_instance(wh: &Value, instance: &str) -> bool {
    let expected_prefix = format!("greentic-webex-{instance}-");
    wh.get("name")
        .and_then(Value::as_str)
        .is_some_and(|name| name.starts_with(&expected_prefix))
}

fn reconcile_webhook(
    api_base: &str,
    token: &str,
    existing: &[Value],
    spec: &WebhookSpec,
    send: &mut HttpSend<'_>,
) -> Value {
    let found = existing.iter().find(|wh| same_owned_webhook(wh, spec));
    match found {
        Some(wh) if existing_matches(wh, spec) => {
            wlog(&format!(
                "reconcile: keeping {} ({}.{}) — already points at the target URL",
                spec.name, spec.resource, spec.event
            ));
            json!({
                "ok": true,
                "action": "keep",
                "webhook_id": wh.get("id").and_then(Value::as_str).unwrap_or_default(),
                "name": spec.name,
                "resource": spec.resource,
                "event": spec.event,
                "filter": spec.filter,
                "targetUrl": spec.target_url,
                "http_status": 200,
            })
        }
        Some(wh) => {
            wlog(&format!(
                "reconcile: {} ({}.{}) exists with a different target URL — recreating",
                spec.name, spec.resource, spec.event
            ));
            let delete = delete_webhook(api_base, token, wh, send);
            if delete.get("ok").and_then(Value::as_bool) != Some(true) {
                return delete;
            }
            create_webhook(api_base, token, spec, "update", send)
        }
        None => {
            wlog(&format!(
                "reconcile: {} ({}.{}) missing — creating",
                spec.name, spec.resource, spec.event
            ));
            create_webhook(api_base, token, spec, "create", send)
        }
    }
}

fn same_owned_webhook(wh: &Value, spec: &WebhookSpec) -> bool {
    wh.get("name").and_then(Value::as_str) == Some(spec.name.as_str())
        && wh.get("resource").and_then(Value::as_str) == Some(spec.resource)
        && wh.get("event").and_then(Value::as_str) == Some(spec.event)
        && value_str(wh, "filter") == spec.filter.as_deref().unwrap_or("")
}

fn existing_matches(wh: &Value, spec: &WebhookSpec) -> bool {
    same_owned_webhook(wh, spec)
        && wh.get("targetUrl").and_then(Value::as_str) == Some(spec.target_url.as_str())
        && secret_matches(wh, spec)
}

/// Webex signs every delivery with the secret the webhook was created with, and
/// returns that secret when the webhook is read. A webhook created by another
/// process (an older instance still booting during a rollout, a previous deploy)
/// can therefore point at the right URL and still sign with a secret this
/// process does not hold: every delivery then fails signature verification and
/// is dropped without a reply. Keeping such a webhook is the bug, so a differing
/// secret makes it stale.
///
/// Nothing to compare when no secret is configured, and a webhook the API
/// returned without one is treated as matching (unknown, not different) so a
/// Webex that stops echoing it does not recreate the webhooks on every start.
fn secret_matches(wh: &Value, spec: &WebhookSpec) -> bool {
    let Some(want) = spec.secret.as_deref().filter(|value| !value.is_empty()) else {
        return true;
    };
    match wh.get("secret").and_then(Value::as_str) {
        Some(have) if !have.is_empty() => have == want,
        _ => true,
    }
}

fn create_webhook(
    api_base: &str,
    token: &str,
    spec: &WebhookSpec,
    action: &str,
    send: &mut HttpSend<'_>,
) -> Value {
    let body = create_webhook_payload(spec);
    let resp = match send(&client::Request {
        method: "POST".to_string(),
        url: format!("{api_base}/webhooks"),
        headers: vec![
            ("Authorization".to_string(), format!("Bearer {token}")),
            ("Content-Type".to_string(), "application/json".to_string()),
        ],
        body: Some(serde_json::to_vec(&body).unwrap_or_default()),
    }) {
        Ok(r) => r,
        Err(e) => {
            wlog(&format!(
                "POST /webhooks for {}.{} transport error: {}",
                spec.resource, spec.event, e.message
            ));
            return json!({
                "ok": false,
                "resource": spec.resource,
                "event": spec.event,
                "error": format!("create {}.{} failed: {}", spec.resource, spec.event, e.message)
            });
        }
    };
    response_result(action, spec, resp)
}

fn delete_webhook(api_base: &str, token: &str, webhook: &Value, send: &mut HttpSend<'_>) -> Value {
    let webhook_id = webhook
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let resource = value_str(webhook, "resource");
    let event = value_str(webhook, "event");
    let name = value_str(webhook, "name");
    let filter = value_str(webhook, "filter");
    let target_url = value_str(webhook, "targetUrl");
    if webhook_id.is_empty() {
        return json!({
            "ok": false,
            "action": "delete",
            "webhook_id": "",
            "name": name,
            "resource": resource,
            "event": event,
            "filter": if filter.is_empty() { Value::Null } else { Value::String(filter.to_string()) },
            "targetUrl": target_url,
            "error": "cannot delete Webex webhook without id",
        });
    }

    let resp = match send(&client::Request {
        method: "DELETE".to_string(),
        url: format!("{api_base}/webhooks/{webhook_id}"),
        headers: vec![("Authorization".to_string(), format!("Bearer {token}"))],
        body: None,
    }) {
        Ok(r) => r,
        Err(e) => {
            wlog(&format!(
                "DELETE /webhooks/{webhook_id} ({resource}.{event}) transport error: {}",
                e.message
            ));
            return json!({
                "ok": false,
                "action": "delete",
                "webhook_id": webhook_id,
                "name": name,
                "resource": resource,
                "event": event,
                "filter": if filter.is_empty() { Value::Null } else { Value::String(filter.to_string()) },
                "targetUrl": target_url,
                "error": format!("delete {resource}.{event} failed: {}", e.message)
            });
        }
    };
    let ok = (200..300).contains(&(resp.status as u32));
    json!({
        "ok": ok,
        "action": "delete",
        "webhook_id": webhook_id,
        "name": name,
        "resource": resource,
        "event": event,
        "filter": if filter.is_empty() { Value::Null } else { Value::String(filter.to_string()) },
        "targetUrl": target_url,
        "http_status": resp.status,
        "webex_response": parse_response_body(resp.body),
    })
}

fn create_webhook_payload(spec: &WebhookSpec) -> Value {
    let mut body = json!({
        "name": spec.name,
        "targetUrl": spec.target_url,
        "resource": spec.resource,
        "event": spec.event,
    });
    if let Some(filter) = spec.filter.as_deref().filter(|value| !value.is_empty()) {
        body["filter"] = json!(filter);
    }
    if let Some(secret) = spec.secret.as_deref().filter(|value| !value.is_empty()) {
        body["secret"] = json!(secret);
    }
    body
}

fn response_result(action: &str, spec: &WebhookSpec, resp: client::Response) -> Value {
    let status = resp.status;
    let body = parse_response_body(resp.body);
    let ok = (200..300).contains(&(status as u32));
    json!({
        "ok": ok,
        "action": action,
        "webhook_id": body.get("id").and_then(Value::as_str).unwrap_or_default(),
        "name": spec.name,
        "resource": spec.resource,
        "event": spec.event,
        "filter": spec.filter,
        "targetUrl": spec.target_url,
        "http_status": status,
        "webex_response": body,
    })
}

fn parse_response_body(body: Option<Vec<u8>>) -> Value {
    body.and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(Value::Null)
}

fn value_str<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

fn sanitize_name_part(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

fn url_segment(input: &str) -> String {
    input
        .bytes()
        .flat_map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                vec![byte as char]
            }
            _ => format!("%{byte:02X}").chars().collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const API_BASE: &str = "https://webexapis.com/v1";
    const TARGET_URL: &str =
        "https://current.example/v1/messaging/ingress/messaging-webex/demo/default";
    const INSTANCE: &str = "demo-default";
    const BOT_PERSON_ID: &str = "bot-person-123";

    #[derive(Debug)]
    struct RecordedRequest {
        method: String,
        url: String,
        body: Value,
    }

    fn response(status: u16, body: Value) -> client::Response {
        client::Response {
            status,
            headers: Vec::new(),
            body: Some(serde_json::to_vec(&body).expect("response body")),
        }
    }

    fn run_setup_with_existing(existing: Vec<Value>) -> (Value, Vec<RecordedRequest>) {
        let mut requests = Vec::new();
        let mut created = 0usize;
        let parsed = json!({});
        let result = {
            let mut send = |request: &client::Request| {
                let body = request
                    .body
                    .as_ref()
                    .and_then(|bytes| serde_json::from_slice(bytes).ok())
                    .unwrap_or(Value::Null);
                requests.push(RecordedRequest {
                    method: request.method.clone(),
                    url: request.url.clone(),
                    body,
                });

                if request.method == "GET" && request.url == format!("{API_BASE}/people/me") {
                    return Ok(response(200, json!({"id": BOT_PERSON_ID})));
                }
                if request.method == "GET" && request.url == format!("{API_BASE}/webhooks") {
                    return Ok(response(200, json!({"items": existing})));
                }
                if request.method == "POST" && request.url == format!("{API_BASE}/webhooks") {
                    created += 1;
                    return Ok(response(200, json!({"id": format!("created-{created}")})));
                }
                if request.method == "DELETE"
                    && request.url.starts_with(&format!("{API_BASE}/webhooks/"))
                {
                    return Ok(response(204, Value::Null));
                }
                Err(client::HostError {
                    code: "unexpected-request".to_string(),
                    message: format!("{} {}", request.method, request.url),
                })
            };

            setup_webhook_with_sender(
                WebhookSetup {
                    api_base: API_BASE,
                    token: "token",
                    target_url: TARGET_URL,
                    instance: INSTANCE,
                    room_id: None,
                    secret: Some("secret"),
                },
                &parsed,
                &mut send,
            )
        };
        (result, requests)
    }

    fn current_webhook(name: &str, resource: &str, event: &str, filter: Option<&str>) -> Value {
        webhook_value(
            &format!("id-{name}"),
            name,
            resource,
            event,
            filter,
            TARGET_URL,
        )
    }

    fn webhook_value(
        id: &str,
        name: &str,
        resource: &str,
        event: &str,
        filter: Option<&str>,
        target_url: &str,
    ) -> Value {
        let mut value = json!({
            "id": id,
            "name": name,
            "resource": resource,
            "event": event,
            "targetUrl": target_url,
        });
        if let Some(filter) = filter {
            value["filter"] = json!(filter);
        }
        value
    }

    #[test]
    fn builds_message_webhooks_for_mentions_and_direct_rooms() {
        let specs = webhook_specs(
            "demo-default",
            "https://example.com/hook",
            None,
            Some("bot-123"),
            Some("secret"),
        );
        let payload = create_webhook_payload(&specs[0]);
        let direct_payload = create_webhook_payload(&specs[1]);

        assert_eq!(
            payload["name"],
            "greentic-webex-demo-default-messages-mentioned-created"
        );
        assert_eq!(payload["targetUrl"], "https://example.com/hook");
        assert_eq!(payload["resource"], "messages");
        assert_eq!(payload["event"], "created");
        assert_eq!(payload["filter"], "mentionedPeople=bot-123");
        assert_eq!(payload["secret"], "secret");

        assert_eq!(
            direct_payload["name"],
            "greentic-webex-demo-default-messages-direct-created"
        );
        assert_eq!(direct_payload["resource"], "messages");
        assert_eq!(direct_payload["event"], "created");
        assert_eq!(direct_payload["filter"], "roomType=direct");
        assert_eq!(direct_payload["secret"], "secret");
    }

    #[test]
    fn builds_single_room_scoped_message_webhook_when_room_configured() {
        let specs = webhook_specs(
            "demo-room",
            "https://example.com/hook",
            Some("room-1"),
            None,
            None,
        );
        let payload = create_webhook_payload(&specs[0]);

        assert_eq!(payload["name"], "greentic-webex-demo-room-messages-created");
        assert_eq!(payload["resource"], "messages");
        assert_eq!(payload["event"], "created");
        assert_eq!(payload["filter"], "roomId=room-1");
    }

    #[test]
    fn builds_attachment_actions_payload_with_room_filter_when_configured() {
        let specs = webhook_specs(
            "demo-room",
            "https://example.com/hook",
            Some("room-1"),
            None,
            None,
        );
        let payload = create_webhook_payload(&specs[1]);

        assert_eq!(
            payload["name"],
            "greentic-webex-demo-room-attachment-actions-created"
        );
        assert_eq!(payload["resource"], "attachmentActions");
        assert_eq!(payload["event"], "created");
        assert_eq!(payload["filter"], "roomId=room-1");
        assert!(payload.get("secret").is_none());
    }

    #[test]
    fn reuses_existing_matching_webhook() {
        let spec = webhook_specs(
            "demo",
            "https://example.com/hook",
            None,
            Some("bot-123"),
            None,
        )
        .remove(0);
        let existing = json!({
            "id": "wh-1",
            "name": spec.name,
            "resource": spec.resource,
            "event": spec.event,
            "filter": spec.filter,
            "targetUrl": spec.target_url
        });

        assert!(same_owned_webhook(&existing, &spec));
        assert!(existing_matches(&existing, &spec));
    }

    #[test]
    fn setup_creates_mention_direct_and_action_webhooks_when_none_exist() {
        let (result, requests) = run_setup_with_existing(Vec::new());

        assert_eq!(result["ok"], true);
        let webhooks = result["webhooks"].as_array().expect("webhook results");
        assert_eq!(webhooks.len(), 3);
        assert!(webhooks.iter().all(|item| item["action"] == "create"));

        let post_bodies: Vec<&Value> = requests
            .iter()
            .filter(|request| request.method == "POST")
            .map(|request| &request.body)
            .collect();
        assert_eq!(post_bodies.len(), 3);
        assert!(post_bodies.iter().any(|body| {
            body["name"] == "greentic-webex-demo-default-messages-mentioned-created"
                && body["resource"] == "messages"
                && body["event"] == "created"
                && body["filter"] == format!("mentionedPeople={BOT_PERSON_ID}")
                && body["targetUrl"] == TARGET_URL
        }));
        assert!(post_bodies.iter().any(|body| {
            body["name"] == "greentic-webex-demo-default-messages-direct-created"
                && body["resource"] == "messages"
                && body["event"] == "created"
                && body["filter"] == "roomType=direct"
                && body["targetUrl"] == TARGET_URL
        }));
        assert!(post_bodies.iter().any(|body| {
            body["name"] == "greentic-webex-demo-default-attachment-actions-created"
                && body["resource"] == "attachmentActions"
                && body["event"] == "created"
                && body.get("filter").is_none()
                && body["targetUrl"] == TARGET_URL
        }));
    }

    #[test]
    fn setup_keeps_existing_current_webhooks_without_duplicates() {
        let existing = vec![
            current_webhook(
                "greentic-webex-demo-default-messages-mentioned-created",
                "messages",
                "created",
                Some(&format!("mentionedPeople={BOT_PERSON_ID}")),
            ),
            current_webhook(
                "greentic-webex-demo-default-messages-direct-created",
                "messages",
                "created",
                Some("roomType=direct"),
            ),
            current_webhook(
                "greentic-webex-demo-default-attachment-actions-created",
                "attachmentActions",
                "created",
                None,
            ),
        ];
        let (result, requests) = run_setup_with_existing(existing);

        assert_eq!(result["ok"], true);
        let webhooks = result["webhooks"].as_array().expect("webhook results");
        assert_eq!(webhooks.len(), 3);
        assert!(webhooks.iter().all(|item| item["action"] == "keep"));
        assert!(!requests.iter().any(|request| request.method == "POST"));
        assert!(!requests.iter().any(|request| request.method == "DELETE"));
    }

    fn with_secret(mut wh: Value, secret: &str) -> Value {
        wh["secret"] = json!(secret);
        wh
    }

    fn three_current_webhooks(secret: Option<&str>) -> Vec<Value> {
        let all = vec![
            current_webhook(
                "greentic-webex-demo-default-messages-mentioned-created",
                "messages",
                "created",
                Some(&format!("mentionedPeople={BOT_PERSON_ID}")),
            ),
            current_webhook(
                "greentic-webex-demo-default-messages-direct-created",
                "messages",
                "created",
                Some("roomType=direct"),
            ),
            current_webhook(
                "greentic-webex-demo-default-attachment-actions-created",
                "attachmentActions",
                "created",
                None,
            ),
        ];
        match secret {
            Some(secret) => all.into_iter().map(|wh| with_secret(wh, secret)).collect(),
            None => all,
        }
    }

    #[test]
    fn setup_recreates_webhooks_whose_signing_secret_differs() {
        // Same names, same target URL, but created with another secret: every
        // delivery would fail signature verification, so they must be replaced.
        let (result, requests) =
            run_setup_with_existing(three_current_webhooks(Some("old-secret")));

        assert_eq!(result["ok"], true);
        let deletes = requests.iter().filter(|r| r.method == "DELETE").count();
        let posts: Vec<&RecordedRequest> = requests.iter().filter(|r| r.method == "POST").collect();
        assert_eq!(deletes, 3, "the three mismatched webhooks are deleted");
        assert_eq!(posts.len(), 3, "and created again");
        assert!(posts.iter().all(|r| r.body["secret"] == "secret"));
    }

    #[test]
    fn setup_keeps_webhooks_whose_signing_secret_matches() {
        let (result, requests) = run_setup_with_existing(three_current_webhooks(Some("secret")));

        assert_eq!(result["ok"], true);
        assert!(
            !requests
                .iter()
                .any(|r| r.method == "POST" || r.method == "DELETE")
        );
    }

    #[test]
    fn a_webhook_listed_without_a_secret_is_not_treated_as_different() {
        // The base fixtures carry no secret field: unknown must not churn.
        let (_, requests) = run_setup_with_existing(three_current_webhooks(None));
        assert!(
            !requests
                .iter()
                .any(|r| r.method == "POST" || r.method == "DELETE")
        );
    }

    #[test]
    fn no_configured_secret_never_counts_as_a_mismatch() {
        let spec = webhook_specs(
            "demo",
            "https://example.com/hook",
            None,
            Some("bot-123"),
            None,
        )
        .remove(0);
        let existing = with_secret(
            webhook_value(
                "wh-1",
                &spec.name,
                spec.resource,
                spec.event,
                spec.filter.as_deref(),
                "https://example.com/hook",
            ),
            "whatever",
        );
        assert!(existing_matches(&existing, &spec));
    }

    #[test]
    fn setup_deletes_stale_greentic_webhooks_and_preserves_unrelated_webhooks() {
        let existing = vec![
            webhook_value(
                "old-mention",
                "greentic-webex-demo-default-messages-created-mentioned",
                "messages",
                "created",
                Some("mentionedPeople=me"),
                "https://old.example/v1/messaging/ingress/messaging-webex/default/default",
            ),
            webhook_value(
                "old-direct",
                "greentic-webex-demo-default-messages-direct-created",
                "messages",
                "created",
                Some("roomType=direct"),
                "https://old.example/v1/messaging/ingress/messaging-webex/default/default",
            ),
            webhook_value(
                "manual",
                "manual-experiment",
                "messages",
                "created",
                None,
                "https://old.example/hook",
            ),
        ];
        let (result, requests) = run_setup_with_existing(existing);

        assert_eq!(result["ok"], true);
        let deletes: Vec<&RecordedRequest> = requests
            .iter()
            .filter(|request| request.method == "DELETE")
            .collect();
        assert_eq!(deletes.len(), 2);
        assert!(
            deletes
                .iter()
                .any(|request| request.url.ends_with("/webhooks/old-mention"))
        );
        assert!(
            deletes
                .iter()
                .any(|request| request.url.ends_with("/webhooks/old-direct"))
        );
        assert!(
            !deletes
                .iter()
                .any(|request| request.url.ends_with("/webhooks/manual"))
        );

        let webhooks = result["webhooks"].as_array().expect("webhook results");
        assert_eq!(
            webhooks
                .iter()
                .filter(|item| item["action"] == "delete")
                .count(),
            2
        );
        assert_eq!(
            webhooks
                .iter()
                .filter(|item| item["action"] == "create")
                .count(),
            3
        );
    }

    #[test]
    fn setup_deletes_duplicate_current_webhooks_but_keeps_one() {
        let duplicate_name = "greentic-webex-demo-default-messages-direct-created";
        let existing = vec![
            webhook_value(
                "direct-1",
                duplicate_name,
                "messages",
                "created",
                Some("roomType=direct"),
                TARGET_URL,
            ),
            webhook_value(
                "direct-2",
                duplicate_name,
                "messages",
                "created",
                Some("roomType=direct"),
                TARGET_URL,
            ),
        ];
        let (result, requests) = run_setup_with_existing(existing);

        assert_eq!(result["ok"], true);
        assert!(requests.iter().any(
            |request| request.method == "DELETE" && request.url.ends_with("/webhooks/direct-2")
        ));
        let webhooks = result["webhooks"].as_array().expect("webhook results");
        assert!(
            webhooks
                .iter()
                .any(|item| { item["name"] == duplicate_name && item["action"] == "keep" })
        );
    }

    #[test]
    fn missing_token_returns_clear_error_without_network() {
        let out: Value = serde_json::from_slice(&setup_webhook(
            br#"{"public_base_url":"https://example.com"}"#,
        ))
        .expect("json");

        assert_eq!(out["ok"], false);
        assert!(
            out["error"]
                .as_str()
                .unwrap_or_default()
                .contains("bot token")
        );
    }

    #[test]
    fn target_url_uses_standard_provider_ingress_route() {
        assert_eq!(
            build_target_url("https://host.example/", "tenant a", "channel/b"),
            "https://host.example/v1/messaging/ingress/messaging-webex/tenant%20a/channel%2Fb"
        );
    }
}
