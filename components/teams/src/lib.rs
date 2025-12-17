#![allow(unsafe_op_in_unsafe_fn)]

mod bindings {
    wit_bindgen::generate!({ path: "wit/teams", world: "teams", generate_all });
}

use bindings::Guest;
use bindings::greentic::http::http_client;
use bindings::greentic::secrets_store::secrets_store;
use serde_json::Value;
use urlencoding::encode;

const GRAPH_MESSAGE_URL: &str = "https://graph.microsoft.com/v1.0";
const MS_GRAPH_TENANT_ID: &str = "MS_GRAPH_TENANT_ID";
const MS_GRAPH_CLIENT_ID: &str = "MS_GRAPH_CLIENT_ID";
const MS_GRAPH_CLIENT_SECRET: &str = "MS_GRAPH_CLIENT_SECRET";
const TOKEN_SCOPE: &str = "https://graph.microsoft.com/.default";

struct Component;

impl Guest for Component {
    fn send_message(destination_json: String, text: String) -> Result<String, String> {
        let dest = parse_destination(&destination_json)?;
        let token = get_access_token()?;

        let url = format!(
            "{}/teams/{}/channels/{}/messages",
            GRAPH_MESSAGE_URL, dest.team_id, dest.channel_id
        );
        let body = format_message_json(&destination_json, &text);

        let req = http_client::Request {
            method: "POST".into(),
            url,
            headers: vec![
                ("Content-Type".into(), "application/json".into()),
                ("Authorization".into(), format!("Bearer {}", token)),
            ],
            body: Some(body.clone().into_bytes()),
        };

        let resp = http_client::send(&req, None)
            .map_err(|e| format!("transport error: {} ({})", e.message, e.code))?;

        if (200..300).contains(&resp.status) {
            Ok(body)
        } else {
            Err(format!(
                "transport error: graph returned status {}",
                resp.status
            ))
        }
    }

    fn handle_webhook(_headers_json: String, body_json: String) -> Result<String, String> {
        let parsed: Value = serde_json::from_str(&body_json)
            .map_err(|_| "validation error: invalid body".to_string())?;
        let normalized = serde_json::json!({"ok": true, "event": parsed});
        serde_json::to_string(&normalized).map_err(|_| "other error: serialization failed".into())
    }

    fn refresh() -> Result<String, String> {
        Ok(r#"{"ok":true,"refresh":"not-needed"}"#.to_string())
    }

    fn format_message(destination_json: String, text: String) -> String {
        format_message_json(&destination_json, &text)
    }
}

fn get_secret(key: &str) -> Result<String, String> {
    match secrets_store::get(key) {
        Ok(Some(bytes)) => String::from_utf8(bytes).map_err(|_| "secret not valid utf-8".into()),
        Ok(None) => Err("secret not found".into()),
        Err(e) => secret_error(e),
    }
}

fn secret_error(error: secrets_store::SecretsError) -> Result<String, String> {
    Err(match error {
        secrets_store::SecretsError::NotFound => "secret not found".into(),
        secrets_store::SecretsError::Denied => "secret access denied".into(),
        secrets_store::SecretsError::InvalidKey => "secret key invalid".into(),
        secrets_store::SecretsError::Internal => "secret lookup failed".into(),
    })
}

fn get_access_token() -> Result<String, String> {
    let tenant_id = get_secret(MS_GRAPH_TENANT_ID)?;
    let client_id = get_secret(MS_GRAPH_CLIENT_ID)?;
    let client_secret = get_secret(MS_GRAPH_CLIENT_SECRET)?;

    let token_url = format!(
        "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
        tenant_id
    );

    let form = format!(
        "client_id={}&client_secret={}&grant_type=client_credentials&scope={}",
        encode(&client_id),
        encode(&client_secret),
        encode(TOKEN_SCOPE)
    );

    let req = http_client::Request {
        method: "POST".into(),
        url: token_url,
        headers: vec![(
            "Content-Type".into(),
            "application/x-www-form-urlencoded".into(),
        )],
        body: Some(form.into_bytes()),
    };

    let resp = http_client::send(&req, None)
        .map_err(|e| format!("transport error: {} ({})", e.message, e.code))?;

    if !(200..300).contains(&resp.status) {
        return Err(format!(
            "transport error: token endpoint returned status {}",
            resp.status
        ));
    }

    let body = resp.body.unwrap_or_default();
    let value: Value = serde_json::from_slice(&body)
        .map_err(|_| "other error: invalid token response".to_string())?;
    let token = value
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| "other error: token response missing access_token".to_string())?;

    Ok(token.to_string())
}

#[derive(Debug)]
struct Destination {
    team_id: String,
    channel_id: String,
}

fn parse_destination(json: &str) -> Result<Destination, String> {
    let value: Value = serde_json::from_str(json)
        .map_err(|_| "validation error: invalid destination json".to_string())?;
    let team_id = value
        .get("team_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "validation error: missing team_id".to_string())?;
    let channel_id = value
        .get("channel_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "validation error: missing channel_id".to_string())?;
    Ok(Destination {
        team_id: team_id.to_string(),
        channel_id: channel_id.to_string(),
    })
}

fn format_message_json(destination_json: &str, text: &str) -> String {
    let fallback = serde_json::json!({"body":{"contentType":"html","content":text}});
    let destination: Value = serde_json::from_str(destination_json).unwrap_or_default();
    let payload = serde_json::json!({
        "to": destination,
        "body": {
            "contentType": "html",
            "content": text
        }
    });
    serde_json::to_string(&payload).unwrap_or_else(|_| fallback.to_string())
}

bindings::__export_world_teams_cabi!(Component with_types_in bindings);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_destination() {
        let dest = parse_destination(r#"{"team_id":"t1","channel_id":"c1"}"#).unwrap();
        assert_eq!(dest.team_id, "t1");
        assert_eq!(dest.channel_id, "c1");
    }

    #[test]
    fn format_message_shape() {
        let json = format_message_json(r#"{"team_id":"t1","channel_id":"c1"}"#, "hello");
        let value: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["to"]["team_id"], "t1");
        assert_eq!(value["body"]["content"], "hello");
        assert_eq!(value["body"]["contentType"], "html");
    }
}
