mod bindings {
    wit_bindgen::generate!({
        path: "wit/messaging-ingress-teams",
        world: "messaging-ingress-teams",
        generate_all
    });
}

#[path = "../../../messaging-teams/src/bot_framework.rs"]
mod bot_framework;

mod setup;
mod teams_pkg;

#[cfg(not(test))]
use base64::{Engine as _, engine::general_purpose};
use bindings::exports::provider::common0_0_2::ingress::Guest as IngressGuest;
use bindings::exports::provider::common0_0_2::subscriptions::Guest as SubscriptionsGuest;
use bindings::exports::provider::common0_0_3::ingress::Guest as ConfiguredIngressGuest;
use bindings::greentic::http::http_client as client;
use bindings::greentic::secrets_store::secrets_store;
use bindings::greentic::state::state_store;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use urlencoding::encode;

const DEFAULT_GRAPH_BASE: &str = "https://graph.microsoft.com/v1.0";
const DEFAULT_AUTH_BASE: &str = "https://login.microsoftonline.com";
const DEFAULT_TOKEN_SCOPE: &str = "https://graph.microsoft.com/.default";
#[cfg(not(test))]
const AZURE_MANAGEMENT_BASE: &str = "https://management.azure.com";
#[cfg(not(test))]
const AZURE_BOT_API_VERSION: &str = "2022-09-15";
#[cfg(not(test))]
const AZURE_RESOURCE_GROUP_API_VERSION: &str = "2021-04-01";
#[cfg(not(test))]
const DEFAULT_CLIENT_ID_KEY: &str = "MS_GRAPH_CLIENT_ID";
const DEFAULT_REFRESH_TOKEN_KEY: &str = "MS_GRAPH_REFRESH_TOKEN";
const STATE_KEY: &str = "messaging.teams.subscriptions";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderConfig {
    tenant_id: String,
    client_id: String,
    #[serde(default)]
    graph_base_url: Option<String>,
    #[serde(default)]
    auth_base_url: Option<String>,
    #[serde(default)]
    token_scope: Option<String>,
}

#[derive(Debug, Clone)]
struct SubscriptionSpec {
    resource: String,
    change_type: String,
    expiration_datetime: Option<String>,
    lifecycle_notification_url: Option<String>,
    client_state: Option<String>,
}

#[derive(Debug, Clone)]
struct ExistingSubscription {
    id: String,
    resource: String,
    change_type: String,
    expiration_datetime: Option<String>,
    notification_url: Option<String>,
}

struct Component;

impl IngressGuest for Component {
    fn handle_webhook(headers_json: String, body_json: String) -> Result<String, String> {
        if let Some((method, path)) = request_method_path(&headers_json)
            && path.contains("/setup/messaging-teams/")
            && let Some(result) = setup::handle(&method, &path, &body_json)
        {
            return result;
        }
        if let Some(token) = validation_token_from_headers(&headers_json) {
            return Ok(token);
        }
        let parsed: Value = serde_json::from_str(&body_json)
            .map_err(|_| "validation error: invalid body".to_string())?;
        if bot_framework::is_bot_framework_activity(&parsed) {
            let normalized = bot_framework::handle_bot_framework_activity(&headers_json, &parsed)?;
            // Record the activity so the setup wizard's first_bot_framework_post resolves.
            if let Some((_, path)) = request_method_path(&headers_json)
                && let Some(tenant) = ingress_tenant_from_path(&path)
            {
                setup::record_activity(&tenant, &parsed);
            }
            return serde_json::to_string(&normalized)
                .map_err(|_| "other error: serialization failed".to_string());
        }
        let expected_client_state = expected_client_state_from_headers(&headers_json);
        let events = normalize_graph_notifications(&parsed, expected_client_state.as_deref())?;
        let normalized = json!({
            "ok": true,
            "provider": "messaging.teams.graph",
            "event": parsed,
            "events": events,
        });
        serde_json::to_string(&normalized)
            .map_err(|_| "other error: serialization failed".to_string())
    }
}

impl ConfiguredIngressGuest for Component {
    fn handle_webhook(
        headers_json: String,
        body_json: String,
        _config_json: String,
    ) -> Result<String, String> {
        <Component as IngressGuest>::handle_webhook(headers_json, body_json)
    }
}

impl SubscriptionsGuest for Component {
    fn sync_subscriptions(config_json: String, state_json: String) -> Result<String, String> {
        let config = parse_config(&config_json)?;
        let state_val = parse_state(&state_json)?;
        let webhook_url = state_val
            .get("webhook_url")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing webhook_url".to_string())?;
        let desired = parse_desired_subscriptions(&state_val)?;
        if desired.is_empty() {
            return Err("no desired_subscriptions provided".into());
        }

        let token = acquire_token(&config)?;
        let mut existing = list_subscriptions(&config, &token)?;
        let mut actions: Vec<Value> = Vec::new();

        for spec in desired {
            if let Some(found) = find_matching(&existing, &spec, webhook_url) {
                if let Some(expiration) = spec.expiration_datetime.clone() {
                    renew_subscription(&config, &token, &found.id, &expiration)?;
                    actions.push(json!({
                        "action": "renewed",
                        "id": found.id,
                        "resource": found.resource,
                        "change_type": found.change_type,
                        "expiration_datetime": expiration,
                    }));
                }
            } else {
                let created = create_subscription(&config, &token, webhook_url, &spec)?;
                actions.push(json!({
                    "action": "created",
                    "id": created.id,
                    "resource": created.resource,
                    "change_type": created.change_type,
                    "expiration_datetime": created.expiration_datetime,
                }));
                existing.push(created);
            }
        }

        let state_out = json!({
            "ok": true,
            "webhook_url": webhook_url,
            "desired_subscriptions": desired_specs_to_json(&state_val),
            "subscriptions": existing_subscriptions_to_json(&existing),
            "actions": actions,
        });

        write_state(&state_out)?;
        serde_json::to_string(&state_out)
            .map_err(|_| "other error: serialization failed".to_string())
    }
}

impl bindings::exports::greentic::provider_schema_core::schema_core_api::Guest for Component {
    fn describe() -> Vec<u8> {
        serde_json::to_vec(&json!({
            "provider_type": "messaging.teams",
            "capabilities": ["setup", "ingress", "subscriptions"],
            "ops": [
                "bot-framework-registration",
                "teams-app-publish",
                "teams-app-install"
            ]
        }))
        .unwrap_or_default()
    }

    fn validate_config(_config_json: Vec<u8>) -> Vec<u8> {
        json_bytes(&json!({"ok": true}))
    }

    fn healthcheck() -> Vec<u8> {
        json_bytes(&json!({"status": "healthy"}))
    }

    fn invoke(op: String, input_json: Vec<u8>) -> Vec<u8> {
        wlog(&format!("wizard step '{op}' started"));
        let output = match op.as_str() {
            "bot-framework-registration" => handle_bot_framework_registration_request(&input_json),
            "teams-app-publish" => handle_teams_app_publish_request(&input_json),
            "teams-app-install" => handle_teams_app_install_request(&input_json),
            other => json!({
                "ok": false,
                "blocked": true,
                "error": format!("unsupported setup op: {other}")
            }),
        };
        if output.get("ok").and_then(Value::as_bool) == Some(true) {
            wlog(&format!(
                "wizard step '{op}' ok (action={})",
                output.get("action").and_then(Value::as_str).unwrap_or("-")
            ));
        } else {
            wlog(&format!(
                "wizard step '{op}' BLOCKED: {}",
                output
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
            ));
        }
        json_bytes(&output)
    }
}

bindings::export!(Component with_types_in bindings);

/// Component-side operational log. The runner host forwards WASM stderr to
/// the operator console, so these lines surface next to the host's own
/// `[setup-action ...]` logs. Never log token/secret VALUES here —
/// presence/absence only.
fn wlog(msg: &str) {
    eprintln!("[messaging-teams-ingress] {msg}");
}

fn json_bytes(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).unwrap_or_default()
}

fn handle_bot_framework_registration_request(input_json: &[u8]) -> Value {
    let request = match serde_json::from_slice::<Value>(input_json) {
        Ok(value) => value,
        Err(_) => {
            return setup_blocked("invalid setup registration request JSON");
        }
    };
    let body = match request.get("body_json").and_then(Value::as_str) {
        Some(raw) => match serde_json::from_str::<Value>(raw) {
            Ok(value) => value,
            Err(_) => return setup_blocked("invalid setup registration body JSON"),
        },
        None => request,
    };
    handle_bot_framework_registration_body(&body)
}

fn setup_request_body(
    input_json: &[u8],
    invalid_request: &str,
    invalid_body: &str,
) -> Result<Value, Value> {
    let request =
        serde_json::from_slice::<Value>(input_json).map_err(|_| setup_blocked(invalid_request))?;
    match request.get("body_json").and_then(Value::as_str) {
        Some(raw) => serde_json::from_str::<Value>(raw).map_err(|_| setup_blocked(invalid_body)),
        None => Ok(request),
    }
}

fn handle_teams_app_publish_request(input_json: &[u8]) -> Value {
    let body = match setup_request_body(
        input_json,
        "invalid Teams app publish request JSON",
        "invalid Teams app publish body JSON",
    ) {
        Ok(value) => value,
        Err(error) => return error,
    };
    handle_teams_app_publish_body(&body)
}

fn handle_teams_app_publish_body(body: &Value) -> Value {
    if body.get("provider_id").and_then(Value::as_str) != Some("messaging-teams") {
        return setup_blocked("provider_id must be messaging-teams");
    }
    let token = match required_body_str(body, "graph_access_token") {
        Ok(value) => value,
        Err(error) => return error,
    };
    let bot_app_id = match required_body_str(body, "bot_app_id") {
        Ok(value) => value,
        Err(error) => return error,
    };
    let display_name = body_str(body, "bot_display_name");
    let version = body_str(body, "teams_app_version");
    match publish_or_reuse_teams_app(token, bot_app_id, version, display_name) {
        Ok(value) => value,
        Err(error) => setup_blocked(&error),
    }
}

#[cfg(not(test))]
fn publish_or_reuse_teams_app(
    token: &str,
    bot_app_id: &str,
    version: &str,
    display_name: &str,
) -> Result<Value, String> {
    let package = teams_pkg::build_package(bot_app_id, bot_app_id, version, display_name);
    let url = format!("{DEFAULT_GRAPH_BASE}/appCatalogs/teamsApps");
    let (action, catalog_app_id) = match graph_setup_zip_request(token, "POST", &url, package) {
        Ok((status, body)) if status < 300 => {
            let catalog_app_id = body
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or(bot_app_id)
                .to_string();
            ("publish", catalog_app_id)
        }
        Ok((409, _)) => (
            "exists",
            lookup_existing_teams_catalog_app(token, bot_app_id)?,
        ),
        Ok((status, body)) => {
            return Err(format!(
                "Teams app catalog publish failed (HTTP {status}): {}",
                graph_setup_error_message(&body, "unknown error")
            ));
        }
        Err(error) => return Err(error),
    };
    Ok(teams_app_publish_result(
        action,
        &catalog_app_id,
        bot_app_id,
        version,
    ))
}

#[cfg(test)]
fn publish_or_reuse_teams_app(
    _token: &str,
    bot_app_id: &str,
    version: &str,
    _display_name: &str,
) -> Result<Value, String> {
    Ok(teams_app_publish_result(
        "publish",
        "catalog-app",
        bot_app_id,
        version,
    ))
}

fn teams_app_publish_result(
    action: &str,
    catalog_app_id: &str,
    bot_app_id: &str,
    version: &str,
) -> Value {
    json!({
        "ok": true,
        "action": action,
        "teams_app_id": catalog_app_id,
        "catalog_app_id": catalog_app_id,
        "external_id": bot_app_id,
        "manifest_version": if version.trim().is_empty() { "1.0.0" } else { version.trim() },
        "add_to_teams_url": format!("https://teams.microsoft.com/l/app/{catalog_app_id}?source=app-details-dialog"),
    })
}

fn handle_teams_app_install_request(input_json: &[u8]) -> Value {
    let body = match setup_request_body(
        input_json,
        "invalid Teams app install request JSON",
        "invalid Teams app install body JSON",
    ) {
        Ok(value) => value,
        Err(error) => return error,
    };
    handle_teams_app_install_body(&body)
}

fn handle_teams_app_install_body(body: &Value) -> Value {
    if body.get("provider_id").and_then(Value::as_str) != Some("messaging-teams") {
        return setup_blocked("provider_id must be messaging-teams");
    }
    let token = match required_body_str(body, "graph_access_token") {
        Ok(value) => value,
        Err(error) => return error,
    };
    let bot_app_id = match required_body_str(body, "bot_app_id") {
        Ok(value) => value,
        Err(error) => return error,
    };
    match install_or_reuse_teams_app(token, bot_app_id) {
        Ok(value) => value,
        Err(error) => setup_blocked(&error),
    }
}

#[cfg(not(test))]
fn install_or_reuse_teams_app(token: &str, bot_app_id: &str) -> Result<Value, String> {
    let catalog_app_id = lookup_existing_teams_catalog_app(token, bot_app_id)?;
    let user_id =
        match graph_setup_json_request(token, "GET", &format!("{DEFAULT_GRAPH_BASE}/me"), None) {
            Ok((status, body)) if status < 300 => body
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| "Microsoft Graph /me response missing user id".to_string())?,
            Ok((status, body)) => {
                return Err(format!(
                    "Microsoft Graph /me failed (HTTP {status}): {}",
                    graph_setup_error_message(&body, "unknown error")
                ));
            }
            Err(error) => return Err(error),
        };
    let url = format!(
        "{DEFAULT_GRAPH_BASE}/users/{}/teamwork/installedApps",
        encode(&user_id)
    );
    let body = json!({
        "teamsApp@odata.bind": format!("{DEFAULT_GRAPH_BASE}/appCatalogs/teamsApps/{catalog_app_id}")
    });
    let action = match graph_setup_json_request(token, "POST", &url, Some(body)) {
        Ok((status, _)) if status < 300 => "install",
        Ok((409, _)) => "exists",
        Ok((status, body)) => {
            return Err(format!(
                "Teams app user install failed (HTTP {status}): {}",
                graph_setup_error_message(&body, "unknown error")
            ));
        }
        Err(error) => return Err(error),
    };
    Ok(teams_app_install_result(
        action,
        &catalog_app_id,
        bot_app_id,
        &user_id,
    ))
}

#[cfg(test)]
fn install_or_reuse_teams_app(_token: &str, bot_app_id: &str) -> Result<Value, String> {
    Ok(teams_app_install_result(
        "install",
        "catalog-app",
        bot_app_id,
        "user-id",
    ))
}

fn teams_app_install_result(
    action: &str,
    catalog_app_id: &str,
    bot_app_id: &str,
    user_id: &str,
) -> Value {
    json!({
        "ok": true,
        "action": action,
        "teams_app_id": catalog_app_id,
        "catalog_app_id": catalog_app_id,
        "installed_for": user_id,
        "add_to_teams_url": format!("https://teams.microsoft.com/l/app/{catalog_app_id}?source=app-details-dialog"),
        "open_bot_chat_url": format!("https://teams.microsoft.com/l/chat/0/0?users=28:{bot_app_id}&message=hello"),
    })
}

pub(crate) fn handle_bot_framework_registration_body(body: &Value) -> Value {
    for key in [
        "tenant",
        "team",
        "bot_app_id",
        "bot_app_password",
        "messaging_endpoint",
        "public_base_url",
        "bot_display_name",
        "azure_management_access_token",
    ] {
        if body
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
        {
            return setup_blocked(&format!("missing {key}"));
        }
    }
    if body.get("provider_id").and_then(Value::as_str) != Some("messaging-teams") {
        return setup_blocked("provider_id must be messaging-teams");
    }
    if body.get("channel").and_then(Value::as_str) != Some("msteams") {
        return setup_blocked("channel must be msteams");
    }
    let messaging_endpoint = body
        .get("messaging_endpoint")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let public_base_url = body
        .get("public_base_url")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !messaging_endpoint.starts_with("https://") {
        return setup_blocked("messaging_endpoint must be an HTTPS URL Teams can reach");
    }
    if !messaging_endpoint.starts_with(public_base_url.trim_end_matches('/')) {
        return setup_blocked("messaging_endpoint must be under public_base_url");
    }
    let tenant = body
        .get("tenant")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let team = body.get("team").and_then(Value::as_str).unwrap_or_default();
    let bot_app_id = body
        .get("bot_app_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    // Best-effort: the Azure Bot resource is not required to advance setup. If it
    // can't be created (e.g. no reachable Azure subscription), record the reason
    // and continue so publish/install/egress can still be exercised.
    let channel_registration =
        match register_microsoft_bot_channel(body, messaging_endpoint, bot_app_id) {
            Ok(value) => value,
            Err(error) => json!({
                "kind": "microsoft_bot_channel_registration",
                "status": "skipped",
                "warning": error,
            }),
        };
    let registration = bot_framework_registration_record(body, messaging_endpoint);
    if let Err(error) = persist_bot_framework_registration(tenant, team, bot_app_id, &registration)
    {
        return setup_blocked(&error);
    }

    json!({
        "ok": true,
        "action": "register",
        "provider_id": "messaging-teams",
        "channel": "msteams",
        "registration": registration,
        "microsoft_bot_channel_registration": channel_registration,
        "tenant": body.get("tenant").cloned().unwrap_or(Value::Null),
        "team": body.get("team").cloned().unwrap_or(Value::Null),
        "target_messaging_endpoint": messaging_endpoint,
    })
}

fn bot_framework_registration_record(body: &Value, messaging_endpoint: &str) -> Value {
    json!({
        "kind": "greentic_bot_framework",
        "status": "registered",
        "provider_id": "messaging-teams",
        "channel": "msteams",
        "tenant": body.get("tenant").cloned().unwrap_or(Value::Null),
        "team": body.get("team").cloned().unwrap_or(Value::Null),
        "bot_app_id": body.get("bot_app_id").cloned().unwrap_or(Value::Null),
        "bot_app_password_ref": "setup_config.bot_app_password",
        "messaging_endpoint": messaging_endpoint,
        "public_base_url": body.get("public_base_url").cloned().unwrap_or(Value::Null),
    })
}

fn body_str<'a>(body: &'a Value, key: &str) -> &'a str {
    body.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn required_body_str<'a>(body: &'a Value, key: &str) -> Result<&'a str, Value> {
    body.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| setup_blocked(&format!("missing {key}")))
}

#[cfg(not(test))]
fn graph_setup_error_message(body: &Value, fallback: &str) -> String {
    body.get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .unwrap_or(fallback)
        .to_string()
}

#[cfg(not(test))]
fn graph_setup_json_request(
    token: &str,
    method: &str,
    url: &str,
    body: Option<Value>,
) -> Result<(u16, Value), String> {
    let mut headers = vec![("Authorization".into(), format!("Bearer {token}"))];
    let body = if let Some(body) = body {
        headers.push(("Content-Type".into(), "application/json".into()));
        Some(
            serde_json::to_vec(&body)
                .map_err(|_| "failed to serialize Microsoft Graph setup request".to_string())?,
        )
    } else {
        None
    };
    let request = client::Request {
        method: method.to_string(),
        url: url.to_string(),
        headers,
        body,
    };
    wlog(&format!("graph {method} {url}"));
    let response = client::send(&request, None, None).map_err(|err| {
        wlog(&format!(
            "graph {method} {url} transport error: {}",
            err.message
        ));
        format!("Microsoft Graph setup request failed: {}", err.message)
    })?;
    let status = response.status as u16;
    let body = response
        .body
        .as_deref()
        .and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok())
        .unwrap_or(Value::Null);
    if status >= 300 {
        wlog(&format!(
            "graph {method} {url} -> HTTP {status}: {}",
            graph_setup_error_message(&body, "no error body")
        ));
    }
    Ok((status, body))
}

#[cfg(not(test))]
fn graph_setup_zip_request(
    token: &str,
    method: &str,
    url: &str,
    body: Vec<u8>,
) -> Result<(u16, Value), String> {
    let request = client::Request {
        method: method.to_string(),
        url: url.to_string(),
        headers: vec![
            ("Authorization".into(), format!("Bearer {token}")),
            ("Content-Type".into(), "application/zip".into()),
        ],
        body: Some(body),
    };
    wlog(&format!("graph {method} {url} (zip package upload)"));
    let response = client::send(&request, None, None).map_err(|err| {
        wlog(&format!(
            "graph {method} {url} transport error: {}",
            err.message
        ));
        format!("Microsoft Graph setup request failed: {}", err.message)
    })?;
    let status = response.status as u16;
    let body = response
        .body
        .as_deref()
        .and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok())
        .unwrap_or(Value::Null);
    if status >= 300 {
        wlog(&format!(
            "graph {method} {url} -> HTTP {status}: {}",
            graph_setup_error_message(&body, "no error body")
        ));
    }
    Ok((status, body))
}

#[cfg(not(test))]
fn lookup_existing_teams_catalog_app(token: &str, external_id: &str) -> Result<String, String> {
    let url = format!(
        "{DEFAULT_GRAPH_BASE}/appCatalogs/teamsApps?$filter=externalId eq '{}'",
        encode(external_id)
    );
    match graph_setup_json_request(token, "GET", &url, None) {
        Ok((status, body)) if status < 300 => body
            .get("value")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(|app| app.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| {
                "existing Teams app catalog id could not be resolved by externalId".to_string()
            }),
        Ok((status, body)) => Err(format!(
            "Teams app catalog lookup failed (HTTP {status}): {}",
            graph_setup_error_message(&body, "unknown error")
        )),
        Err(error) => Err(error),
    }
}

#[cfg(not(test))]
fn azure_arm_request(
    token: &str,
    method: &str,
    url: &str,
    body: Option<Value>,
) -> Result<(u16, Value), String> {
    let mut headers = vec![("Authorization".into(), format!("Bearer {token}"))];
    let body =
        if let Some(body) = body {
            headers.push(("Content-Type".into(), "application/json".into()));
            Some(serde_json::to_vec(&body).map_err(|_| {
                "failed to serialize Microsoft bot registration request".to_string()
            })?)
        } else {
            None
        };
    let request = client::Request {
        method: method.to_string(),
        url: url.to_string(),
        headers,
        body,
    };
    let response = client::send(&request, None, None).map_err(|err| {
        format!(
            "Microsoft bot channel registration request failed: {}",
            err.message
        )
    })?;
    let status = response.status as u16;
    let body = response
        .body
        .as_deref()
        .and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok())
        .unwrap_or(Value::Null);
    Ok((status, body))
}

#[cfg(not(test))]
fn azure_error_message(body: &Value, fallback: &str) -> String {
    if let Some(message) = body
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
    {
        return message.to_string();
    }
    if body.is_null() {
        return fallback.to_string();
    }
    // Surface the raw API response when it doesn't match the standard error shape.
    serde_json::to_string(body).unwrap_or_else(|_| fallback.to_string())
}

fn azure_safe_name(raw: &str) -> String {
    let mut out = String::new();
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' {
            out.push(ch.to_ascii_lowercase());
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "greentic-teams-bot".to_string()
    } else {
        out.chars().take(42).collect()
    }
}

#[cfg(not(test))]
fn first_non_empty<'a>(values: &[&'a str]) -> &'a str {
    values
        .iter()
        .copied()
        .find(|value| !value.trim().is_empty())
        .unwrap_or_default()
}

#[cfg(not(test))]
fn looks_like_guid(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    for (idx, byte) in bytes.iter().enumerate() {
        if matches!(idx, 8 | 13 | 18 | 23) {
            if *byte != b'-' {
                return false;
            }
        } else if !byte.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

#[cfg(not(test))]
fn jwt_claim_str(token: &str, claim: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let bytes = general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| general_purpose::URL_SAFE.decode(payload))
        .ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    value
        .get(claim)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
}

#[cfg(not(test))]
fn azure_tenant_id_for_single_tenant_bot(body: &Value, token: &str) -> Result<String, String> {
    let configured = body_str(body, "azure_auth_tenant").trim();
    if looks_like_guid(configured) {
        return Ok(configured.to_string());
    }
    if let Some(tid) = jwt_claim_str(token, "tid")
        && looks_like_guid(&tid)
    {
        return Ok(tid);
    }
    Err(
        "Microsoft bot channel registration requires a tenant id for SingleTenant bots. Set Azure auth tenant to your Microsoft Entra tenant GUID and retry."
            .to_string(),
    )
}

#[cfg(not(test))]
fn register_microsoft_bot_channel(
    body: &Value,
    messaging_endpoint: &str,
    bot_app_id: &str,
) -> Result<Value, String> {
    let token = body_str(body, "azure_management_access_token");
    let mut actions = Vec::new();
    let subscription_id = if body_str(body, "azure_subscription_id").trim().is_empty() {
        let url = format!("{AZURE_MANAGEMENT_BASE}/subscriptions?api-version=2022-12-01");
        match azure_arm_request(token, "GET", &url, None) {
            Ok((status, body)) if status < 300 => {
                let items = body
                    .get("value")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let chosen = items
                    .iter()
                    .find(|item| {
                        item.get("state")
                            .and_then(Value::as_str)
                            .map(|state| {
                                state.eq_ignore_ascii_case("enabled")
                                    || state.eq_ignore_ascii_case("warned")
                            })
                            .unwrap_or(true)
                    })
                    .or_else(|| items.first())
                    .ok_or_else(|| {
                        format!(
                            "Azure subscription discovery returned no subscriptions (HTTP {status}): {}",
                            serde_json::to_string(&body).unwrap_or_else(|_| "<unparseable body>".to_string())
                        )
                    })?;
                let id = chosen
                    .get("subscriptionId")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        "Azure subscription response did not include subscriptionId".to_string()
                    })?
                    .to_string();
                actions.push(json!({
                    "action": "selected_subscription",
                    "subscription_id": id,
                    "display_name": chosen.get("displayName").cloned().unwrap_or(Value::Null),
                    "candidates": items.len(),
                }));
                id
            }
            Ok((status, body)) => {
                return Err(format!(
                    "Azure subscription discovery failed (HTTP {status}): {}",
                    azure_error_message(&body, "unknown error")
                ));
            }
            Err(error) => return Err(error),
        }
    } else {
        body_str(body, "azure_subscription_id").to_string()
    };
    let resource_group = if body_str(body, "azure_resource_group").trim().is_empty() {
        discover_or_default_resource_group(token, &subscription_id, &mut actions)?
    } else {
        body_str(body, "azure_resource_group").to_string()
    };
    let resource_group_location = first_non_empty(&[
        body_str(body, "azure_resource_group_location"),
        "westeurope",
    ]);
    let location = first_non_empty(&[body_str(body, "azure_location"), "global"]);
    let bot_name = azure_safe_name(first_non_empty(&[
        body_str(body, "azure_bot_name"),
        body_str(body, "bot_display_name"),
        bot_app_id,
        "greentic-teams-bot",
    ]));
    if body_str(body, "azure_bot_name").trim().is_empty() {
        actions.push(json!({
            "action": "derived_bot_name",
            "azure_bot_name": bot_name,
        }));
    }
    let display_name = first_non_empty(&[body_str(body, "bot_display_name"), &bot_name]);
    let azure_tenant_id = azure_tenant_id_for_single_tenant_bot(body, token)?;

    let rg_url = format!(
        "{AZURE_MANAGEMENT_BASE}/subscriptions/{}/resourcegroups/{}?api-version={AZURE_RESOURCE_GROUP_API_VERSION}",
        encode(&subscription_id),
        encode(&resource_group)
    );
    ensure_microsoft_bot_resource_group(
        token,
        &rg_url,
        &resource_group,
        resource_group_location,
        &mut actions,
    )?;

    let bot_url = format!(
        "{AZURE_MANAGEMENT_BASE}/subscriptions/{}/resourceGroups/{}/providers/Microsoft.BotService/botServices/{}?api-version={AZURE_BOT_API_VERSION}",
        encode(&subscription_id),
        encode(&resource_group),
        encode(&bot_name)
    );
    let mut bot_properties = Map::new();
    bot_properties.insert("displayName".to_string(), json!(display_name));
    bot_properties.insert("endpoint".to_string(), json!(messaging_endpoint));
    bot_properties.insert("msaAppId".to_string(), json!(bot_app_id));
    bot_properties.insert("msaAppType".to_string(), json!("SingleTenant"));
    bot_properties.insert("isCmekEnabled".to_string(), json!(false));
    bot_properties.insert("publicNetworkAccess".to_string(), json!("Enabled"));
    bot_properties.insert("msaAppTenantId".to_string(), json!(azure_tenant_id));
    let bot_body = json!({
        "location": location,
        "sku": { "name": "F0" },
        "kind": "azurebot",
        "properties": bot_properties
    });
    match azure_arm_request(token, "PUT", &bot_url, Some(bot_body)) {
        Ok((status, _)) if status < 300 => {}
        Ok((status, body)) => {
            return Err(format!(
                "Microsoft bot channel registration create/update failed (HTTP {status}): {}",
                azure_error_message(&body, "unknown error")
            ));
        }
        Err(error) => return Err(error),
    }

    let channel_url = format!(
        "{AZURE_MANAGEMENT_BASE}/subscriptions/{}/resourceGroups/{}/providers/Microsoft.BotService/botServices/{}/channels/MsTeamsChannel?api-version={AZURE_BOT_API_VERSION}",
        encode(&subscription_id),
        encode(&resource_group),
        encode(&bot_name)
    );
    let channel_body = json!({
        "location": location,
        "properties": {
            "channelName": "MsTeamsChannel",
            "properties": {
                "isEnabled": true,
                "acceptedTerms": true
            }
        }
    });
    match azure_arm_request(token, "PUT", &channel_url, Some(channel_body)) {
        Ok((status, _)) if status < 300 => {}
        Ok((status, body)) => {
            return Err(format!(
                "Microsoft Teams bot channel enable failed (HTTP {status}): {}",
                azure_error_message(&body, "unknown error")
            ));
        }
        Err(error) => return Err(error),
    }

    Ok(json!({
        "kind": "microsoft_bot_channel_registration",
        "status": "registered",
        "bot_name": bot_name,
        "resource_group": resource_group,
        "subscription_id": subscription_id,
        "channel": "msteams",
        "endpoint": messaging_endpoint,
        "actions": actions,
    }))
}

#[cfg(not(test))]
fn discover_or_default_resource_group(
    token: &str,
    subscription_id: &str,
    actions: &mut Vec<Value>,
) -> Result<String, String> {
    let default_group = "greentic-bots";
    let groups_url = format!(
        "{AZURE_MANAGEMENT_BASE}/subscriptions/{}/resourcegroups?api-version={AZURE_RESOURCE_GROUP_API_VERSION}",
        encode(subscription_id)
    );
    match azure_arm_request(token, "GET", &groups_url, None) {
        Ok((status, body)) if status < 300 => {
            let items = body
                .get("value")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let chosen = items
                .iter()
                .find(|item| {
                    item.get("name")
                        .and_then(Value::as_str)
                        .map(|name| name.eq_ignore_ascii_case(default_group))
                        .unwrap_or(false)
                })
                .or_else(|| {
                    items.iter().find(|item| {
                        item.get("name")
                            .and_then(Value::as_str)
                            .map(|name| name.to_ascii_lowercase().contains("greentic"))
                            .unwrap_or(false)
                    })
                })
                .or_else(|| items.first());
            if let Some(item) = chosen
                && let Some(name) = item.get("name").and_then(Value::as_str)
            {
                actions.push(json!({
                    "action": "selected_resource_group",
                    "resource_group": name,
                    "candidates": items.len(),
                }));
                return Ok(name.to_string());
            }
        }
        Ok((403, body)) => {
            actions.push(json!({
                "action": "defaulted_resource_group",
                "resource_group": default_group,
                "reason": "resource group listing was denied; continuing with the default name",
                "discovery_error": azure_error_message(&body, "unknown error"),
            }));
            return Ok(default_group.to_string());
        }
        Ok((status, body)) => {
            return Err(format!(
                "Azure resource group discovery failed (HTTP {status}): {}",
                azure_error_message(&body, "unknown error")
            ));
        }
        Err(error) => return Err(error),
    }
    actions.push(json!({
        "action": "defaulted_resource_group",
        "resource_group": default_group,
        "reason": "no existing resource groups were available",
    }));
    Ok(default_group.to_string())
}

#[cfg(not(test))]
fn ensure_microsoft_bot_resource_group(
    token: &str,
    rg_url: &str,
    resource_group: &str,
    location: &str,
    actions: &mut Vec<Value>,
) -> Result<(), String> {
    match azure_arm_request(token, "GET", rg_url, None) {
        Ok((status, _)) if status < 300 => {
            actions.push(json!({
                "action": "selected_resource_group",
                "resource_group": resource_group,
            }));
            return Ok(());
        }
        Ok((404, _)) => {}
        Ok((403, _)) => {
            actions.push(json!({
                "action": "assumed_resource_group",
                "resource_group": resource_group,
                "reason": "resource group lookup was denied; continuing with configured/default name"
            }));
            return Ok(());
        }
        Ok((status, body)) => {
            return Err(format!(
                "Microsoft bot registration resource group lookup failed (HTTP {status}): {}",
                azure_error_message(&body, "unknown error")
            ));
        }
        Err(error) => return Err(error),
    }

    match azure_arm_request(token, "PUT", rg_url, Some(json!({ "location": location }))) {
        Ok((status, _)) if status < 300 => {
            actions.push(json!({
                "action": "created_resource_group",
                "resource_group": resource_group,
                "location": location,
            }));
            Ok(())
        }
        Ok((403, body)) => Err(format!(
            "Microsoft bot registration resource group {resource_group} does not exist or cannot be created. Use an existing resource group in Azure resource group, or grant Microsoft.Resources/subscriptions/resourcegroups/write. Azure said: {}",
            azure_error_message(&body, "unknown error")
        )),
        Ok((status, body)) => Err(format!(
            "Microsoft bot registration resource group create failed (HTTP {status}): {}",
            azure_error_message(&body, "unknown error")
        )),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
fn register_microsoft_bot_channel(
    body: &Value,
    messaging_endpoint: &str,
    _bot_app_id: &str,
) -> Result<Value, String> {
    Ok(json!({
        "kind": "microsoft_bot_channel_registration",
        "status": "registered",
        "bot_name": azure_safe_name(body_str(body, "azure_bot_name")),
        "channel": "msteams",
        "endpoint": messaging_endpoint,
    }))
}

#[cfg(not(test))]
fn bot_framework_registration_key(tenant: &str, team: &str) -> String {
    format!(
        "messaging.teams.bot_framework.registration.{}.{}",
        state_key_part(tenant),
        state_key_part(team)
    )
}

#[cfg(not(test))]
fn bot_framework_app_registration_key(bot_app_id: &str) -> String {
    format!(
        "messaging.teams.bot_framework.app.{}",
        state_key_part(bot_app_id)
    )
}

#[cfg(not(test))]
fn state_key_part(raw: &str) -> String {
    let out: String = raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() {
        "default".to_string()
    } else {
        out
    }
}

#[cfg(not(test))]
fn persist_bot_framework_registration(
    tenant: &str,
    team: &str,
    bot_app_id: &str,
    registration: &Value,
) -> Result<(), String> {
    let bytes = serde_json::to_vec(registration)
        .map_err(|_| "failed to serialize bot framework registration".to_string())?;
    state_store::write(&bot_framework_registration_key(tenant, team), &bytes, None)
        .map_err(|error| format!("bot framework registration state error: {}", error.message))?;
    let app_index = json!({
        "kind": "greentic_bot_framework_app_registration",
        "bot_app_id": bot_app_id,
        "registration_key": bot_framework_registration_key(tenant, team),
        "tenant": tenant,
        "team": team,
        "messaging_endpoint": registration.get("messaging_endpoint").cloned().unwrap_or(Value::Null),
    });
    let app_bytes = serde_json::to_vec(&app_index)
        .map_err(|_| "failed to serialize bot framework app registration".to_string())?;
    state_store::write(
        &bot_framework_app_registration_key(bot_app_id),
        &app_bytes,
        None,
    )
    .map_err(|error| {
        format!(
            "bot framework app registration state error: {}",
            error.message
        )
    })?;
    Ok(())
}

#[cfg(test)]
fn persist_bot_framework_registration(
    _tenant: &str,
    _team: &str,
    _bot_app_id: &str,
    _registration: &Value,
) -> Result<(), String> {
    Ok(())
}

fn setup_blocked(error: &str) -> Value {
    json!({
        "ok": false,
        "blocked": true,
        "error": error
    })
}

fn parse_config(config_json: &str) -> Result<ProviderConfig, String> {
    if config_json.trim().is_empty() {
        return Err("config_json required".into());
    }
    serde_json::from_str::<ProviderConfig>(config_json).map_err(|e| format!("invalid config: {e}"))
}

fn parse_state(state_json: &str) -> Result<Value, String> {
    if state_json.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str::<Value>(state_json).map_err(|_| "invalid state_json".to_string())
}

fn parse_desired_subscriptions(state: &Value) -> Result<Vec<SubscriptionSpec>, String> {
    let desired = state
        .get("desired_subscriptions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    desired
        .into_iter()
        .map(|entry| {
            let resource = entry
                .get("resource")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .or_else(|| build_subscription_resource(&entry))
                .ok_or_else(|| {
                    "desired_subscriptions.resource or team_id/channel_id/chat_id required"
                        .to_string()
                })?;
            let change_type = entry
                .get("change_type")
                .and_then(Value::as_str)
                .unwrap_or("created");
            let expiration_datetime = entry
                .get("expiration_datetime")
                .and_then(Value::as_str)
                .map(|s| s.to_string());
            let lifecycle_notification_url = entry
                .get("lifecycle_notification_url")
                .and_then(Value::as_str)
                .map(|s| s.to_string());
            let client_state = entry
                .get("client_state")
                .and_then(Value::as_str)
                .map(|s| s.to_string());
            Ok(SubscriptionSpec {
                resource,
                change_type: change_type.to_string(),
                expiration_datetime,
                lifecycle_notification_url,
                client_state,
            })
        })
        .collect()
}

fn desired_specs_to_json(state: &Value) -> Value {
    state
        .get("desired_subscriptions")
        .cloned()
        .unwrap_or_else(|| json!([]))
}

fn existing_subscriptions_to_json(existing: &[ExistingSubscription]) -> Value {
    let list: Vec<Value> = existing
        .iter()
        .map(|sub| {
            json!({
                "id": sub.id,
                "resource": sub.resource,
                "change_type": sub.change_type,
                "expiration_datetime": sub.expiration_datetime,
                "notification_url": sub.notification_url,
            })
        })
        .collect();
    Value::Array(list)
}

fn request_method_path(headers_json: &str) -> Option<(String, String)> {
    let headers: Value = serde_json::from_str(headers_json).ok()?;
    let method = headers.get("method").and_then(Value::as_str)?.to_string();
    let path = headers.get("path").and_then(Value::as_str)?.to_string();
    Some((method, path))
}

/// Extract the tenant from `/v1/messaging/ingress/messaging-teams/{tenant}/{team}`.
fn ingress_tenant_from_path(path: &str) -> Option<String> {
    let rest = path.split("/ingress/messaging-teams/").nth(1)?;
    let tenant = rest.trim_start_matches('/').split('/').next()?;
    if tenant.is_empty() {
        None
    } else {
        Some(tenant.to_string())
    }
}

fn validation_token_from_headers(headers_json: &str) -> Option<String> {
    let headers: Value = serde_json::from_str(headers_json).ok()?;
    for key in ["validationToken", "validation_token"] {
        if let Some(token) = headers.get(key).and_then(Value::as_str)
            && !token.is_empty()
        {
            return Some(token.to_string());
        }
    }
    for key in ["query", "query_string", "raw_query"] {
        if let Some(query) = headers.get(key).and_then(Value::as_str)
            && let Some(token) = query_param_value(query, "validationToken")
        {
            return Some(token);
        }
    }
    if let Some(url) = headers.get("url").and_then(Value::as_str)
        && let Some((_, query)) = url.split_once('?')
        && let Some(token) = query_param_value(query, "validationToken")
    {
        return Some(token);
    }
    None
}

fn expected_client_state_from_headers(headers_json: &str) -> Option<String> {
    let headers: Value = serde_json::from_str(headers_json).ok()?;
    headers
        .get("expected_client_state")
        .or_else(|| headers.get("client_state"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn query_param_value(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|part| {
        let (key, value) = part.split_once('=')?;
        if key == name {
            Some(urlencoding::decode(value).ok()?.into_owned())
        } else {
            None
        }
    })
}

fn normalize_graph_notifications(
    body: &Value,
    expected_client_state: Option<&str>,
) -> Result<Value, String> {
    let notifications = body
        .get("value")
        .and_then(Value::as_array)
        .ok_or_else(|| "validation error: Graph notification value[] required".to_string())?;
    let mut events = Vec::new();
    for notification in notifications {
        if let Some(expected) = expected_client_state {
            let actual = notification
                .get("clientState")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if actual != expected {
                return Err("validation error: Graph clientState mismatch".to_string());
            }
        }
        if let Some(event) = normalize_notification(notification) {
            events.push(event);
        }
    }
    Ok(Value::Array(events))
}

fn normalize_notification(notification: &Value) -> Option<Value> {
    let resource = notification
        .get("resource")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let resolved_resource_data;
    let notification_tenant_id = notification.get("tenantId").and_then(Value::as_str);
    let resource_data = match notification.get("resourceData") {
        Some(data) if message_has_body(data) => data,
        Some(data) => {
            resolved_resource_data = resolve_graph_message(resource, notification_tenant_id)
                .unwrap_or_else(|| data.clone());
            &resolved_resource_data
        }
        None => {
            resolved_resource_data =
                resolve_graph_message(resource, notification_tenant_id).unwrap_or(Value::Null);
            &resolved_resource_data
        }
    };
    let ids = parse_graph_resource(resource);
    let message_id = resource_data
        .get("id")
        .and_then(Value::as_str)
        .or(ids.message_id.as_deref())
        .unwrap_or_default();
    let body = resource_data.get("body").unwrap_or(&Value::Null);
    let content = body
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let content_type = body
        .get("contentType")
        .and_then(Value::as_str)
        .unwrap_or("text");
    let text = if content_type.eq_ignore_ascii_case("html") {
        strip_html(content)
    } else {
        content.to_string()
    };
    if should_skip_message(resource_data, &text) {
        return None;
    }
    let destination = if let Some(chat_id) = ids.chat_id.as_ref() {
        Some((chat_id.clone(), "chat"))
    } else if let (Some(team_id), Some(channel_id)) =
        (ids.team_id.as_ref(), ids.channel_id.as_ref())
    {
        Some((format!("{team_id}:{channel_id}"), "channel"))
    } else {
        None
    };
    let mut metadata = Map::new();
    insert_string(&mut metadata, "graph_resource", resource);
    insert_string(
        &mut metadata,
        "change_type",
        notification
            .get("changeType")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    insert_string(
        &mut metadata,
        "subscription_id",
        notification
            .get("subscriptionId")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    insert_string(
        &mut metadata,
        "tenant_id",
        notification
            .get("tenantId")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    insert_optional_string(&mut metadata, "team_id", ids.team_id.as_deref());
    insert_optional_string(&mut metadata, "channel_id", ids.channel_id.as_deref());
    insert_optional_string(&mut metadata, "chat_id", ids.chat_id.as_deref());
    insert_string(
        &mut metadata,
        "webUrl",
        resource_data
            .get("webUrl")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    insert_string(
        &mut metadata,
        "replyToId",
        resource_data
            .get("replyToId")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    insert_string(&mut metadata, "body_content", content);
    insert_string(&mut metadata, "body_content_type", content_type);

    let provider_message_id = if message_id.is_empty() {
        None
    } else {
        Some(format!("teams:{message_id}"))
    };
    let session_id = destination
        .as_ref()
        .map(|(id, _)| id.clone())
        .or_else(|| ids.chat_id.clone())
        .or_else(|| ids.channel_id.clone())
        .or_else(|| ids.team_id.clone())
        .unwrap_or_else(|| "teams".to_string());
    let envelope_id = provider_message_id
        .clone()
        .unwrap_or_else(|| format!("teams:{session_id}"));

    let mut event = Map::new();
    event.insert("id".to_string(), Value::String(envelope_id));
    event.insert(
        "tenant".to_string(),
        json!({
            "env": "default",
            "tenant": "default",
            "tenant_id": "default",
            "attempt": 0
        }),
    );
    event.insert("channel".to_string(), Value::String("teams".to_string()));
    event.insert("session_id".to_string(), Value::String(session_id));
    if !message_id.is_empty() {
        metadata.insert(
            "provider_message_id".to_string(),
            Value::String(format!("teams:{message_id}")),
        );
        metadata.insert(
            "message_id".to_string(),
            Value::String(message_id.to_string()),
        );
        event.insert(
            "provider_message_id".to_string(),
            Value::String(format!("teams:{message_id}")),
        );
        event.insert(
            "message_id".to_string(),
            Value::String(message_id.to_string()),
        );
    }
    metadata.insert(
        "provider".to_string(),
        Value::String("messaging.teams.graph".to_string()),
    );
    metadata.insert("source".to_string(), Value::String("teams".to_string()));
    event.insert("text".to_string(), Value::String(text));
    insert_optional_actor(&mut event, "from", graph_from_id(resource_data).as_deref());
    insert_optional_destination(&mut event, "to", destination.as_ref());
    event.insert("metadata".to_string(), Value::Object(metadata));
    Some(Value::Object(event))
}

fn message_has_body(resource_data: &Value) -> bool {
    resource_data
        .get("body")
        .and_then(|body| body.get("content"))
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|content| !content.is_empty())
}

fn should_skip_message(resource_data: &Value, text: &str) -> bool {
    if has_card_attachment(resource_data) {
        return true;
    }
    text.trim().is_empty()
}

fn has_card_attachment(resource_data: &Value) -> bool {
    resource_data
        .get("attachments")
        .and_then(Value::as_array)
        .is_some_and(|attachments| {
            attachments.iter().any(|attachment| {
                attachment
                    .get("contentType")
                    .and_then(Value::as_str)
                    .is_some_and(|content_type| content_type.contains("card."))
            })
        })
}

#[cfg(test)]
fn resolve_graph_message(_resource: &str, _tenant_id: Option<&str>) -> Option<Value> {
    None
}

#[cfg(not(test))]
fn resolve_graph_message(resource: &str, tenant_id: Option<&str>) -> Option<Value> {
    let resource = resource.trim().trim_start_matches('/');
    if resource.is_empty() {
        return None;
    }
    let client_id = get_secret_any_case(DEFAULT_CLIENT_ID_KEY).ok()?;
    let cfg = ProviderConfig {
        tenant_id: tenant_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| get_secret_any_case("MS_GRAPH_TENANT_ID").ok())?,
        client_id,
        graph_base_url: None,
        auth_base_url: None,
        token_scope: None,
    };
    let token = acquire_token(&cfg).ok()?;
    let url = format!("{}/{}", DEFAULT_GRAPH_BASE, resource);
    let request = client::Request {
        method: "GET".into(),
        url,
        headers: vec![("Authorization".into(), format!("Bearer {}", token))],
        body: None,
    };
    let resp = client::send(&request, None, None).ok()?;
    if resp.status < 200 || resp.status >= 300 {
        return None;
    }
    let body = resp.body.unwrap_or_default();
    serde_json::from_slice(&body).ok()
}

fn insert_optional_actor(map: &mut Map<String, Value>, key: &str, id: Option<&str>) {
    let Some(id) = id.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    map.insert(
        key.to_string(),
        json!({
            "id": id,
            "kind": "user"
        }),
    );
}

fn insert_optional_destination(
    map: &mut Map<String, Value>,
    key: &str,
    destination: Option<&(String, &str)>,
) {
    let Some((id, kind)) = destination else {
        return;
    };
    let id = id.trim();
    if id.is_empty() {
        return;
    }
    map.insert(
        key.to_string(),
        json!([{
            "id": id,
            "kind": kind
        }]),
    );
}

#[derive(Default)]
struct ResourceIds {
    team_id: Option<String>,
    channel_id: Option<String>,
    chat_id: Option<String>,
    message_id: Option<String>,
}

fn parse_graph_resource(resource: &str) -> ResourceIds {
    let parts: Vec<&str> = resource.trim_matches('/').split('/').collect();
    let mut ids = ResourceIds::default();
    let mut i = 0;
    while i < parts.len() {
        match parts[i] {
            segment if graph_segment_id(segment, "teams").is_some() => {
                ids.team_id = graph_segment_id(segment, "teams");
                i += 1;
            }
            segment if graph_segment_id(segment, "channels").is_some() => {
                ids.channel_id = graph_segment_id(segment, "channels");
                i += 1;
            }
            segment if graph_segment_id(segment, "chats").is_some() => {
                ids.chat_id = graph_segment_id(segment, "chats");
                i += 1;
            }
            segment if graph_segment_id(segment, "messages").is_some() => {
                ids.message_id = graph_segment_id(segment, "messages");
                i += 1;
            }
            "teams" if i + 1 < parts.len() => {
                ids.team_id = Some(parts[i + 1].to_string());
                i += 2;
            }
            "channels" if i + 1 < parts.len() => {
                ids.channel_id = Some(parts[i + 1].to_string());
                i += 2;
            }
            "chats" if i + 1 < parts.len() => {
                ids.chat_id = Some(parts[i + 1].to_string());
                i += 2;
            }
            "messages" if i + 1 < parts.len() => {
                ids.message_id = Some(parts[i + 1].to_string());
                i += 2;
            }
            "replies" if i + 1 < parts.len() => {
                ids.message_id = Some(parts[i + 1].to_string());
                i += 2;
            }
            _ => i += 1,
        }
    }
    ids
}

fn graph_segment_id(segment: &str, name: &str) -> Option<String> {
    let prefix = format!("{name}('");
    segment
        .strip_prefix(&prefix)
        .and_then(|rest| rest.strip_suffix("')"))
        .map(ToOwned::to_owned)
        .filter(|value| !value.is_empty())
}

fn graph_from_id(resource_data: &Value) -> Option<String> {
    let from = resource_data.get("from")?;
    for path in [
        &["user", "id"][..],
        &["application", "id"],
        &["device", "id"],
        &["conversation", "id"],
    ] {
        let mut current = from;
        for key in path {
            current = current.get(*key)?;
        }
        if let Some(id) = current.as_str().map(str::trim).filter(|id| !id.is_empty()) {
            return Some(id.to_string());
        }
    }
    None
}

fn insert_string(map: &mut Map<String, Value>, key: &str, value: &str) {
    map.insert(key.to_string(), Value::String(value.to_string()));
}

fn insert_optional_string(map: &mut Map<String, Value>, key: &str, value: Option<&str>) {
    if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
        map.insert(key.to_string(), Value::String(value.to_string()));
    }
}

fn strip_html(input: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for ch in input.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
}

fn build_subscription_resource(entry: &Value) -> Option<String> {
    let chat_id = entry
        .get("chat_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(chat_id) = chat_id {
        return Some(chat_message_resource(chat_id));
    }
    let team_id = entry
        .get("team_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let channel_id = entry
        .get("channel_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    if let Some(message_id) = entry
        .get("message_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(channel_reply_resource(team_id, channel_id, message_id))
    } else {
        Some(channel_message_resource(team_id, channel_id))
    }
}

fn channel_message_resource(team_id: &str, channel_id: &str) -> String {
    format!("/teams/{team_id}/channels/{channel_id}/messages")
}

fn channel_reply_resource(team_id: &str, channel_id: &str, message_id: &str) -> String {
    format!("/teams/{team_id}/channels/{channel_id}/messages/{message_id}/replies")
}

fn chat_message_resource(chat_id: &str) -> String {
    format!("/chats/{chat_id}/messages")
}

fn find_matching<'a>(
    existing: &'a [ExistingSubscription],
    desired: &SubscriptionSpec,
    webhook_url: &str,
) -> Option<&'a ExistingSubscription> {
    existing.iter().find(|sub| {
        sub.resource == desired.resource
            && sub.change_type == desired.change_type
            && sub
                .notification_url
                .as_ref()
                .map(|url| url == webhook_url)
                .unwrap_or(false)
    })
}

fn acquire_token(cfg: &ProviderConfig) -> Result<String, String> {
    let auth_base = cfg
        .auth_base_url
        .clone()
        .unwrap_or_else(|| DEFAULT_AUTH_BASE.to_string());
    let token_url = format!("{}/{}/oauth2/v2.0/token", auth_base, cfg.tenant_id);
    let scope = cfg
        .token_scope
        .clone()
        .unwrap_or_else(|| DEFAULT_TOKEN_SCOPE.to_string());

    let refresh_token = get_secret(DEFAULT_REFRESH_TOKEN_KEY).map_err(|_| {
        "MS_GRAPH_REFRESH_TOKEN is required for Teams Graph subscriptions".to_string()
    })?;
    let form = format!(
        "client_id={}&grant_type=refresh_token&refresh_token={}&scope={}",
        encode(&cfg.client_id),
        encode(&refresh_token),
        encode(&scope)
    );
    send_token_request(&token_url, &form)
}

fn send_token_request(url: &str, form: &str) -> Result<String, String> {
    let request = client::Request {
        method: "POST".into(),
        url: url.to_string(),
        headers: vec![(
            "Content-Type".into(),
            "application/x-www-form-urlencoded".into(),
        )],
        body: Some(form.as_bytes().to_vec()),
    };

    let resp = client::send(&request, None, None)
        .map_err(|e| format!("transport error: {}", e.message))?;
    if resp.status < 200 || resp.status >= 300 {
        return Err(format!("token endpoint returned status {}", resp.status));
    }
    let body = resp.body.unwrap_or_default();
    let json: Value =
        serde_json::from_slice(&body).map_err(|e| format!("invalid token response: {e}"))?;
    let token = json
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| "token response missing access_token".to_string())?;
    Ok(token.to_string())
}

fn list_subscriptions(
    cfg: &ProviderConfig,
    token: &str,
) -> Result<Vec<ExistingSubscription>, String> {
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
        .map_err(|e| format!("transport error: {}", e.message))?;
    if resp.status < 200 || resp.status >= 300 {
        return Err(graph_status_error(
            "list subscriptions",
            resp.status,
            resp.body,
        ));
    }
    let body = resp.body.unwrap_or_default();
    let json: Value = serde_json::from_slice(&body)
        .map_err(|e| format!("invalid subscriptions response: {e}"))?;
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

fn create_subscription(
    cfg: &ProviderConfig,
    token: &str,
    webhook_url: &str,
    spec: &SubscriptionSpec,
) -> Result<ExistingSubscription, String> {
    let graph_base = cfg
        .graph_base_url
        .clone()
        .unwrap_or_else(|| DEFAULT_GRAPH_BASE.to_string());
    let url = format!("{}/subscriptions", graph_base);
    let expiration = spec
        .expiration_datetime
        .clone()
        .ok_or_else(|| "desired_subscriptions.expiration_datetime required".to_string())?;

    let mut payload = json!({
        "changeType": spec.change_type,
        "notificationUrl": webhook_url,
        "lifecycleNotificationUrl": spec
            .lifecycle_notification_url
            .as_deref()
            .unwrap_or(webhook_url),
        "resource": spec.resource,
        "expirationDateTime": expiration,
    });
    if let Some(client_state) = spec.client_state.as_ref() {
        payload
            .as_object_mut()
            .expect("payload object")
            .insert("clientState".into(), Value::String(client_state.clone()));
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
        .map_err(|e| format!("transport error: {}", e.message))?;
    if resp.status < 200 || resp.status >= 300 {
        return Err(graph_status_error(
            "create subscription",
            resp.status,
            resp.body,
        ));
    }
    let body = resp.body.unwrap_or_default();
    let json: Value =
        serde_json::from_slice(&body).map_err(|e| format!("invalid create response: {e}"))?;
    let id = json
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| "create response missing id".to_string())?;
    Ok(ExistingSubscription {
        id: id.to_string(),
        resource: spec.resource.clone(),
        change_type: spec.change_type.clone(),
        expiration_datetime: json
            .get("expirationDateTime")
            .and_then(Value::as_str)
            .map(|s| s.to_string())
            .or(Some(expiration)),
        notification_url: Some(webhook_url.to_string()),
    })
}

fn renew_subscription(
    cfg: &ProviderConfig,
    token: &str,
    subscription_id: &str,
    expiration: &str,
) -> Result<(), String> {
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
        .map_err(|e| format!("transport error: {}", e.message))?;
    if resp.status < 200 || resp.status >= 300 {
        return Err(graph_status_error(
            "renew subscription",
            resp.status,
            resp.body,
        ));
    }
    Ok(())
}

fn graph_status_error(action: &str, status: u16, body: Option<Vec<u8>>) -> String {
    let body = body.unwrap_or_default();
    let body_text = String::from_utf8_lossy(&body).trim().to_string();
    if body_text.is_empty() {
        format!("{action} status {status}")
    } else {
        format!("{action} status {status}: {body_text}")
    }
}

fn write_state(state: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec(state).map_err(|_| "invalid state payload".to_string())?;
    state_store::write(STATE_KEY, &bytes, None)
        .map_err(|e| format!("state store error: {}", e.message))
        .map(|_| ())
}

fn get_secret(key: &str) -> Result<String, String> {
    match secrets_store::get(key) {
        Ok(Some(bytes)) => String::from_utf8(bytes).map_err(|_| format!("secret {key} not utf-8")),
        Ok(None) => Err(format!("missing secret: {key}")),
        Err(e) => Err(format!("secret store error: {e:?}")),
    }
}

#[cfg(not(test))]
fn get_secret_any_case(uppercase: &str) -> Result<String, String> {
    get_secret(uppercase).or_else(|_| get_secret(&uppercase.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_config_applies_optional_defaults_later() {
        let cfg = parse_config(r#"{"tenant_id":"tenant","client_id":"client"}"#).expect("config");

        assert_eq!(cfg.tenant_id, "tenant");
        assert_eq!(cfg.client_id, "client");
        assert!(cfg.graph_base_url.is_none());
    }

    #[test]
    fn parse_desired_subscriptions_defaults_change_type() {
        let state = json!({
            "desired_subscriptions": [
                {
                    "resource": "teams/team-1/channels/channel-1/messages",
                    "expiration_datetime": "2026-01-01T00:00:00Z",
                    "lifecycle_notification_url": "https://example.com/lifecycle"
                }
            ]
        });

        let desired = parse_desired_subscriptions(&state).expect("desired");

        assert_eq!(desired.len(), 1);
        assert_eq!(desired[0].change_type, "created");
        assert_eq!(
            desired[0].resource,
            "teams/team-1/channels/channel-1/messages"
        );
        assert_eq!(
            desired[0].lifecycle_notification_url.as_deref(),
            Some("https://example.com/lifecycle")
        );
    }

    #[test]
    fn parse_state_accepts_empty_and_rejects_invalid_json() {
        assert_eq!(parse_state("").expect("empty state"), json!({}));
        assert_eq!(
            parse_state("{").expect_err("invalid state"),
            "invalid state_json"
        );
    }

    #[test]
    fn parse_desired_subscriptions_requires_resource() {
        let state = json!({
            "desired_subscriptions": [
                {"change_type": "updated"}
            ]
        });

        assert_eq!(
            parse_desired_subscriptions(&state).expect_err("resource required"),
            "desired_subscriptions.resource or team_id/channel_id/chat_id required"
        );
    }

    #[test]
    fn parse_desired_subscriptions_builds_channel_and_chat_resources() {
        let state = json!({
            "desired_subscriptions": [
                {"team_id": "team-1", "channel_id": "channel-1"},
                {"chat_id": "chat-1"}
            ]
        });

        let desired = parse_desired_subscriptions(&state).expect("desired");

        assert_eq!(
            desired[0].resource,
            "/teams/team-1/channels/channel-1/messages"
        );
        assert_eq!(desired[1].resource, "/chats/chat-1/messages");
    }

    #[test]
    fn find_matching_requires_same_resource_change_type_and_webhook() {
        let desired = SubscriptionSpec {
            resource: "users/me/messages".to_string(),
            change_type: "created".to_string(),
            expiration_datetime: None,
            lifecycle_notification_url: None,
            client_state: None,
        };
        let existing = vec![
            ExistingSubscription {
                id: "wrong-url".to_string(),
                resource: desired.resource.clone(),
                change_type: desired.change_type.clone(),
                expiration_datetime: None,
                notification_url: Some("https://old.example/hook".to_string()),
            },
            ExistingSubscription {
                id: "match".to_string(),
                resource: desired.resource.clone(),
                change_type: desired.change_type.clone(),
                expiration_datetime: None,
                notification_url: Some("https://new.example/hook".to_string()),
            },
        ];

        let found = find_matching(&existing, &desired, "https://new.example/hook")
            .expect("matching subscription");

        assert_eq!(found.id, "match");
    }

    #[test]
    fn existing_subscriptions_to_json_preserves_graph_fields() {
        let subscriptions = vec![ExistingSubscription {
            id: "sub-1".to_string(),
            resource: "teams/t/channels/c/messages".to_string(),
            change_type: "created".to_string(),
            expiration_datetime: Some("2026-01-01T00:00:00Z".to_string()),
            notification_url: Some("https://chat.example.com/hook".to_string()),
        }];

        let out = existing_subscriptions_to_json(&subscriptions);

        assert_eq!(out[0]["id"], "sub-1");
        assert_eq!(out[0]["resource"], "teams/t/channels/c/messages");
        assert_eq!(out[0]["notification_url"], "https://chat.example.com/hook");
    }

    #[test]
    fn bot_framework_registration_setup_op_accepts_required_body() {
        let out = handle_bot_framework_registration_body(&json!({
            "provider_id": "messaging-teams",
            "channel": "msteams",
            "tenant": "demo",
            "team": "default",
            "bot_app_id": "bot-app",
            "bot_app_password": "secret",
            "bot_display_name": "Greentic Bot",
            "public_base_url": "https://runtime.example.test",
            "messaging_endpoint": "https://runtime.example.test/v1/messaging/ingress/messaging-teams/demo/default",
            "azure_management_access_token": "management-token",
            "azure_subscription_id": "subscription",
            "azure_resource_group": "greentic-bots",
            "azure_resource_group_location": "westeurope",
            "azure_location": "global",
            "azure_bot_name": "Greentic Teams Bot"
        }));

        assert_eq!(out["ok"], true);
        assert_eq!(
            out["target_messaging_endpoint"],
            "https://runtime.example.test/v1/messaging/ingress/messaging-teams/demo/default"
        );
        assert_eq!(out["registration"]["kind"], "greentic_bot_framework");
        assert_eq!(out["registration"]["status"], "registered");
        assert_eq!(out["registration"]["bot_app_id"], "bot-app");
        assert_eq!(
            out["registration"]["messaging_endpoint"],
            "https://runtime.example.test/v1/messaging/ingress/messaging-teams/demo/default"
        );
        assert!(out["registration"].get("bot_app_password").is_none());
        assert_eq!(
            out["registration"]["bot_app_password_ref"],
            "setup_config.bot_app_password"
        );
        assert_eq!(
            out["microsoft_bot_channel_registration"]["kind"],
            "microsoft_bot_channel_registration"
        );
        assert_eq!(
            out["microsoft_bot_channel_registration"]["bot_name"],
            "greenticteamsbot"
        );
    }

    #[test]
    fn bot_framework_registration_setup_op_blocks_missing_body_fields() {
        let out = handle_bot_framework_registration_body(&json!({
            "provider_id": "messaging-teams",
            "channel": "msteams"
        }));

        assert_eq!(out["ok"], false);
        assert_eq!(out["blocked"], true);
        assert_eq!(out["error"], "missing tenant");
    }

    #[test]
    fn graph_status_error_includes_response_body() {
        let err = graph_status_error(
            "create subscription",
            400,
            Some(br#"{"error":{"message":"lifecycleNotificationUrl is required"}}"#.to_vec()),
        );

        assert!(err.contains("create subscription status 400"));
        assert!(err.contains("lifecycleNotificationUrl is required"));
    }

    #[test]
    fn configured_export_matches_the_unconfigured_one() {
        let headers = r#"{"query":"validationToken=hello%20graph"}"#;
        let plain = <Component as IngressGuest>::handle_webhook(headers.into(), String::new());
        let configured = <Component as ConfiguredIngressGuest>::handle_webhook(
            headers.into(),
            String::new(),
            "null".into(),
        );
        assert_eq!(plain, configured);
    }

    #[test]
    fn webhook_returns_validation_token_plain_text() {
        let out = <Component as IngressGuest>::handle_webhook(
            r#"{"query":"validationToken=hello%20graph"}"#.to_string(),
            String::new(),
        )
        .expect("token");

        assert_eq!(out, "hello graph");
    }

    #[test]
    fn webhook_normalizes_graph_channel_notification() {
        let out = <Component as IngressGuest>::handle_webhook(
            r#"{"expected_client_state":"state-1"}"#.to_string(),
            json!({
                "value": [{
                    "subscriptionId": "sub-1",
                    "clientState": "state-1",
                    "changeType": "created",
                    "tenantId": "tenant-1",
                    "resource": "/teams/team-1/channels/channel-1/messages/message-1",
                    "resourceData": {
                        "id": "message-1",
                        "body": {
                            "contentType": "html",
                            "content": "<b>hello</b>"
                        },
                        "from": {"user": {"id": "user-1"}},
                        "webUrl": "https://teams.example/message"
                    }
                }]
            })
            .to_string(),
        )
        .expect("normalized");
        let parsed: Value = serde_json::from_str(&out).expect("json");

        assert_eq!(parsed["ok"], true);
        assert_eq!(parsed["events"][0]["text"], "hello");
        let envelope: greentic_types::ChannelMessageEnvelope =
            serde_json::from_value(parsed["events"][0].clone()).expect("channel envelope");
        assert_eq!(envelope.id, "teams:message-1");
        assert_eq!(envelope.channel, "teams");
        assert_eq!(envelope.session_id, "team-1:channel-1");
        assert_eq!(envelope.text.as_deref(), Some("hello"));
        assert_eq!(
            parsed["events"][0]["provider_message_id"],
            "teams:message-1"
        );
        assert_eq!(parsed["events"][0]["to"][0]["id"], "team-1:channel-1");
        assert_eq!(parsed["events"][0]["to"][0]["kind"], "channel");
        assert_eq!(parsed["events"][0]["from"]["id"], "user-1");
        assert_eq!(parsed["events"][0]["from"]["kind"], "user");
        assert_eq!(parsed["events"][0]["metadata"]["team_id"], "team-1");
        assert_eq!(parsed["events"][0]["metadata"]["channel_id"], "channel-1");
    }

    #[test]
    fn webhook_omits_unknown_sender_and_parses_quoted_graph_resource_ids() {
        let out = <Component as IngressGuest>::handle_webhook(
            "{}".to_string(),
            json!({
                "value": [{
                    "subscriptionId": "sub-1",
                    "changeType": "created",
                    "tenantId": "tenant-1",
                    "resource": "teams('team-1')/channels('19:channel@thread.tacv2')/messages('1780340545252')",
                    "resourceData": {
                        "id": "1780340545252",
                        "body": {
                            "contentType": "text",
                            "content": "hello"
                        }
                    }
                }]
            })
            .to_string(),
        )
        .expect("normalized");
        let parsed: Value = serde_json::from_str(&out).expect("json");
        let event = parsed["events"][0].as_object().expect("event object");

        assert!(!event.contains_key("from"));
        assert_eq!(event["to"][0]["id"], "team-1:19:channel@thread.tacv2");
        assert_eq!(event["to"][0]["kind"], "channel");
        assert_eq!(event["metadata"]["team_id"], "team-1");
        assert_eq!(event["metadata"]["channel_id"], "19:channel@thread.tacv2");
        assert_eq!(event["provider_message_id"], "teams:1780340545252");
        let envelope: greentic_types::ChannelMessageEnvelope =
            serde_json::from_value(parsed["events"][0].clone()).expect("channel envelope");
        assert_eq!(envelope.id, "teams:1780340545252");
        assert_eq!(envelope.session_id, "team-1:19:channel@thread.tacv2");
    }

    #[test]
    fn webhook_skips_unresolved_empty_notifications() {
        let out = <Component as IngressGuest>::handle_webhook(
            "{}".to_string(),
            json!({
                "value": [{
                    "subscriptionId": "sub-1",
                    "changeType": "created",
                    "tenantId": "tenant-1",
                    "resource": "teams('team-1')/channels('channel-1')/messages('message-1')",
                    "resourceData": {
                        "id": "message-1"
                    }
                }]
            })
            .to_string(),
        )
        .expect("normalized");
        let parsed: Value = serde_json::from_str(&out).expect("json");

        assert_eq!(parsed["events"].as_array().expect("events").len(), 0);
    }

    #[test]
    fn webhook_skips_card_attachment_notifications_to_prevent_echo_loop() {
        let out = <Component as IngressGuest>::handle_webhook(
            "{}".to_string(),
            json!({
                "value": [{
                    "subscriptionId": "sub-1",
                    "changeType": "created",
                    "tenantId": "tenant-1",
                    "resource": "/teams/team-1/channels/channel-1/messages/message-1",
                    "resourceData": {
                        "id": "message-1",
                        "body": {
                            "contentType": "html",
                            "content": "<attachment id=\"greentic-adaptive-card-1\"></attachment>"
                        },
                        "attachments": [{
                            "id": "greentic-adaptive-card-1",
                            "contentType": "application/vnd.microsoft.card.adaptive",
                            "content": "{}"
                        }]
                    }
                }]
            })
            .to_string(),
        )
        .expect("normalized");
        let parsed: Value = serde_json::from_str(&out).expect("json");

        assert_eq!(parsed["events"].as_array().expect("events").len(), 0);
    }

    #[test]
    fn webhook_rejects_client_state_mismatch() {
        let err = <Component as IngressGuest>::handle_webhook(
            r#"{"expected_client_state":"expected"}"#.to_string(),
            json!({
                "value": [{
                    "clientState": "actual",
                    "resource": "/chats/chat-1/messages/message-1"
                }]
            })
            .to_string(),
        )
        .expect_err("mismatch");

        assert!(err.contains("clientState mismatch"));
    }

    #[test]
    fn resource_builders_match_graph_paths() {
        assert_eq!(
            channel_message_resource("team", "channel"),
            "/teams/team/channels/channel/messages"
        );
        assert_eq!(
            channel_reply_resource("team", "channel", "message"),
            "/teams/team/channels/channel/messages/message/replies"
        );
        assert_eq!(chat_message_resource("chat"), "/chats/chat/messages");
    }
}
