#![allow(dead_code)]

use greentic_interfaces_guest::component::node::{InvokeResult, NodeError};
use greentic_interfaces_guest::component_entrypoint;
use greentic_interfaces_guest::state_store;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;

#[derive(Debug, Deserialize)]
struct ApplyInput {
    plan: Plan,
    #[serde(default)]
    dry_run: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct Plan {
    actions: Vec<Action>,
}

#[derive(Debug, Deserialize)]
struct Action {
    #[serde(rename = "type")]
    action_type: String,
    scope: String,
    key: String,
    value: String,
}

#[derive(Debug, Serialize)]
struct ApplyResult {
    ok: bool,
    dry_run: bool,
    actions: Vec<ActionResult>,
    summary: Summary,
}

#[derive(Debug, Serialize)]
struct ActionResult {
    action_type: String,
    scope: String,
    key: String,
    status: String,
    message: Option<String>,
}

#[derive(Debug, Serialize, Default)]
struct Summary {
    config_keys_written: Vec<String>,
    secret_keys_written: Vec<String>,
}

component_entrypoint!({
    manifest: describe_payload,
    invoke: handle_message,
    invoke_stream: false,
});

fn err_string(err: impl std::fmt::Display) -> String {
    format!("{err}")
}

fn describe_payload() -> String {
    serde_json::json!({
        "component": {
            "name": "provision",
            "org": "ai.greentic",
            "version": "0.1.0",
            "world": "greentic:component/component@0.5.0"
        }
    })
    .to_string()
}

fn handle_message(operation: String, input: String) -> InvokeResult {
    match operation.as_str() {
        "apply" => invoke_operation(apply(input)),
        other => InvokeResult::Err(NodeError {
            code: "UNKNOWN_OPERATION".to_string(),
            message: format!("unsupported operation {other}"),
            retryable: false,
            backoff_ms: None,
            details: None,
        }),
    }
}

fn invoke_operation(result: Result<String, String>) -> InvokeResult {
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

fn apply(plan_json: String) -> Result<String, String> {
    let input: ApplyInput = serde_json::from_str(&plan_json).map_err(err_string)?;
    let dry_run = resolve_dry_run(input.dry_run.as_ref());
    let mut actions_out = Vec::new();
    let mut summary = Summary::default();

    for action in input.plan.actions {
        let mut result = ActionResult {
            action_type: action.action_type.clone(),
            scope: action.scope.clone(),
            key: action.key.clone(),
            status: "ok".to_string(),
            message: None,
        };

        let action_type = action.action_type.as_str();
        let op_result = match action_type {
            "config.set" => write_state_value(
                &config_state_key(&action.scope, &action.key),
                &action.value,
                dry_run,
            ),
            "secrets.put" => write_state_value(
                &secrets_state_key(&action.scope, &action.key),
                &action.value,
                dry_run,
            ),
            other => Err(format!("unsupported action type {other}")),
        };

        if let Err(message) = op_result {
            result.status = "error".to_string();
            result.message = Some(message);
            actions_out.push(result);
            continue;
        }

        match action_type {
            "config.set" => summary.config_keys_written.push(action.key.clone()),
            "secrets.put" => summary.secret_keys_written.push(action.key.clone()),
            _ => {}
        }

        actions_out.push(result);
    }

    let ok = actions_out.iter().all(|action| action.status == "ok");
    let result = ApplyResult {
        ok,
        dry_run,
        actions: actions_out,
        summary,
    };
    serde_json::to_string(&result).map_err(err_string)
}

fn resolve_dry_run(input: Option<&Value>) -> bool {
    if let Ok(value) = env::var("PROVISION_DRY_RUN") {
        let value = value.trim().to_ascii_lowercase();
        if matches!(value.as_str(), "1" | "true" | "yes" | "y") {
            return true;
        }
        if matches!(value.as_str(), "0" | "false" | "no" | "n") {
            return false;
        }
    }
    match input {
        Some(Value::Bool(value)) => *value,
        Some(Value::String(raw)) => matches!(
            raw.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "y"
        ),
        _ => false,
    }
}

fn config_state_key(scope: &str, key: &str) -> String {
    format!("config/{scope}/{key}")
}

fn secrets_state_key(scope: &str, key: &str) -> String {
    format!("secrets/{scope}/{key}")
}

fn write_state_value(key: &str, value: &str, dry_run: bool) -> Result<(), String> {
    if dry_run {
        return Ok(());
    }
    state_store::write(key, value.as_bytes(), None)
        .map(|_| ())
        .map_err(|err| format!("state write error: {}", err.message))
}
