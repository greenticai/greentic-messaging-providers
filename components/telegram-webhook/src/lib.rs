#![allow(dead_code)]

use greentic_interfaces_guest::component::node::{InvokeResult, NodeError};
use greentic_interfaces_guest::component_entrypoint;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

mod bindings {
    wit_bindgen::generate!({
        path: "wit/telegram-webhook",
        world: "telegram-webhook",
        generate_all
    });
}

use bindings::greentic::http::client;
use bindings::greentic::secrets_store::secrets_store;

const DEFAULT_API_BASE: &str = "https://api.telegram.org";
const DEFAULT_WEBHOOK_PATH: &str = "";
const TOKEN_SECRET: &str = "TELEGRAM_BOT_TOKEN";

#[derive(Deserialize)]
struct ReconcileInput {
    public_base_url: String,
    #[serde(default)]
    webhook_path: Option<String>,
    #[serde(default)]
    api_base_url: Option<String>,
    #[serde(default)]
    secret_token: Option<String>,
    #[serde(default)]
    dry_run: Option<bool>,
}

#[derive(Serialize)]
struct ReconcileOutput {
    ok: bool,
    expected_url: String,
    current_url: Option<String>,
    final_url: Option<String>,
    webhook_reconciled: bool,
    set_attempted: bool,
    set_skipped_dry_run: bool,
    set_response: Option<Value>,
    webhook_info: Value,
}

component_entrypoint!({
    manifest: describe_manifest,
    invoke: handle_message,
    invoke_stream: false,
});

fn describe_manifest() -> String {
    include_str!("../component.manifest.json").to_string()
}

fn handle_message(operation: String, input: String) -> InvokeResult {
    match operation.as_str() {
        "reconcile_webhook" => invoke(reconcile_webhook(&input)),
        _ => InvokeResult::Err(NodeError {
            code: "UNKNOWN_OPERATION".to_string(),
            message: format!("unsupported operation {operation}"),
            retryable: false,
            backoff_ms: None,
            details: None,
        }),
    }
}

fn invoke(result: Result<String, String>) -> InvokeResult {
    match result {
        Ok(value) => InvokeResult::Ok(value),
        Err(message) => InvokeResult::Err(NodeError {
            code: "INVALID_INPUT".to_string(),
            message,
            retryable: false,
            backoff_ms: None,
            details: None,
        }),
    }
}

fn reconcile_webhook(input: &str) -> Result<String, String> {
    let parsed: ReconcileInput =
        serde_json::from_str(input).map_err(|err| format!("invalid input: {err}"))?;
    let base = parsed.public_base_url.trim();
    if base.is_empty() {
        return Err("public_base_url is required".to_string());
    }
    let webhook_path = parsed
        .webhook_path
        .as_deref()
        .unwrap_or(DEFAULT_WEBHOOK_PATH)
        .trim();
    let expected_url = join_url(base, webhook_path);
    let api_base = parsed
        .api_base_url
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(DEFAULT_API_BASE);
    let token = load_token()?;
    let current_info = get_webhook_info(api_base, &token)?;
    let current_url = extract_url(&current_info);
    let mut set_response = None;
    let mut set_attempted = false;
    let mut set_skipped = false;
    if current_url.as_deref() != Some(&expected_url) {
        if parsed.dry_run.unwrap_or(false) {
            set_skipped = true;
        } else {
            let response = set_webhook(
                api_base,
                &token,
                &expected_url,
                parsed.secret_token.as_deref(),
            )?;
            set_response = Some(response);
            set_attempted = true;
        }
    }
    let final_info = get_webhook_info(api_base, &token)?;
    let final_url = extract_url(&final_info);
    let webhook_reconciled = final_url.as_deref() == Some(&expected_url);
    let output = ReconcileOutput {
        ok: true,
        expected_url,
        current_url,
        final_url,
        webhook_reconciled,
        set_attempted,
        set_skipped_dry_run: set_skipped,
        set_response,
        webhook_info: final_info,
    };
    serde_json::to_string(&output).map_err(|err| format!("serialization failed: {err}"))
}

fn join_url(base: &str, path: &str) -> String {
    let mut base = base.trim_end_matches('/').to_string();
    let trimmed = path.trim();
    if !trimmed.is_empty() {
        if trimmed.starts_with('/') {
            base.push_str(trimmed);
        } else {
            base.push('/');
            base.push_str(trimmed);
        }
    }
    base
}

fn load_token() -> Result<String, String> {
    match secrets_store::get(TOKEN_SECRET) {
        Ok(Some(bytes)) => String::from_utf8(bytes).map_err(|_| "bot token not utf-8".to_string()),
        Ok(None) => Err(format!("missing secret: {TOKEN_SECRET}")),
        Err(err) => Err(format!("secret store error: {err:?}")),
    }
}

fn get_webhook_info(api_base: &str, token: &str) -> Result<Value, String> {
    let url = format!("{api_base}/bot{token}/getWebhookInfo");
    let request = client::Request {
        method: "GET".into(),
        url,
        headers: vec![],
        body: None,
    };
    let response = send_request(&request)?;
    ensure_ok(&response, "getWebhookInfo")?;
    Ok(response)
}

fn set_webhook(
    api_base: &str,
    token: &str,
    expected_url: &str,
    secret_token: Option<&str>,
) -> Result<Value, String> {
    let mut payload = json!({ "url": expected_url });
    if let Some(secret) = secret_token.filter(|s| !s.trim().is_empty()) {
        payload.as_object_mut().expect("payload object").insert(
            "secret_token".to_string(),
            Value::String(secret.to_string()),
        );
    }
    let request = client::Request {
        method: "POST".into(),
        url: format!("{api_base}/bot{token}/setWebhook"),
        headers: vec![("Content-Type".into(), "application/json".into())],
        body: Some(
            serde_json::to_vec(&payload)
                .map_err(|err| format!("payload serialization failed: {err}"))?,
        ),
    };
    let response = send_request(&request)?;
    ensure_ok(&response, "setWebhook")?;
    Ok(response)
}

fn send_request(request: &client::Request) -> Result<Value, String> {
    let resp = client::send(request, None, None).map_err(|err| err.message.clone())?;
    if resp.status < 200 || resp.status >= 300 {
        return Err(format!("telegram returned status {}", resp.status));
    }
    let body = resp.body.as_deref().unwrap_or(&[]);
    serde_json::from_slice(body).map_err(|err| format!("invalid json: {err}"))
}

fn ensure_ok(body: &Value, operation: &str) -> Result<(), String> {
    if body.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        return Ok(());
    }
    let description = body
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown error");
    Err(format!("{operation} failed: {description}"))
}

fn extract_url(body: &Value) -> Option<String> {
    body.get("result")
        .and_then(|result| result.get("url"))
        .and_then(|url| url.as_str())
        .map(|s| s.to_string())
}
