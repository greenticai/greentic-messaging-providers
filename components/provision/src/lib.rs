use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;

mod bindings {
    wit_bindgen::generate!({
        path: "wit/provision",
        world: "provision",
        generate_all
    });
}

use bindings::Guest;
use bindings::greentic::state::state_store;

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

struct Component;

impl Guest for Component {
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
}

bindings::__export_world_provision_cabi!(Component with_types_in bindings);

fn err_string(err: impl std::fmt::Display) -> String {
    format!("{err}")
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
