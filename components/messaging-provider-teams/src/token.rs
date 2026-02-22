use serde_json::Value;
use urlencoding::encode as url_encode;

use crate::bindings::greentic::http::http_client as client;
use crate::config::{ProviderConfig, get_secret};
use crate::{DEFAULT_AUTH_BASE, DEFAULT_CLIENT_SECRET_KEY, DEFAULT_REFRESH_TOKEN_KEY, DEFAULT_TOKEN_SCOPE};

pub(crate) fn acquire_token(cfg: &ProviderConfig) -> Result<String, String> {
    let auth_base = cfg
        .auth_base_url
        .clone()
        .unwrap_or_else(|| DEFAULT_AUTH_BASE.to_string());
    let token_url = format!("{}/{}/oauth2/v2.0/token", auth_base, cfg.tenant_id);
    let scope = cfg
        .token_scope
        .clone()
        .unwrap_or_else(|| DEFAULT_TOKEN_SCOPE.to_string());

    let refresh_token = cfg
        .refresh_token
        .clone()
        .or_else(|| get_secret(DEFAULT_REFRESH_TOKEN_KEY).ok());
    if let Some(refresh_token) = refresh_token {
        let mut form = format!(
            "client_id={}&grant_type=refresh_token&refresh_token={}&scope={}",
            url_encode(&cfg.client_id),
            url_encode(&refresh_token),
            url_encode(&scope)
        );
        let client_secret = cfg
            .client_secret
            .clone()
            .or_else(|| get_secret(DEFAULT_CLIENT_SECRET_KEY).ok());
        if let Some(secret) = client_secret {
            form.push_str(&format!("&client_secret={}", url_encode(&secret)));
        }
        return send_token_request(&token_url, &form);
    }

    let client_secret = cfg
        .client_secret
        .clone()
        .or_else(|| get_secret(DEFAULT_CLIENT_SECRET_KEY).ok())
        .ok_or_else(|| "missing client_secret (config or secret store)".to_string())?;
    let form = format!(
        "client_id={}&client_secret={}&grant_type=client_credentials&scope={}",
        url_encode(&cfg.client_id),
        url_encode(&client_secret),
        url_encode(&scope)
    );
    send_token_request(&token_url, &form)
}

pub(crate) fn send_token_request(url: &str, form: &str) -> Result<String, String> {
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
