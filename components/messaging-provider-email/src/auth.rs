use crate::bindings::greentic::http::http_client as client;
use crate::bindings::greentic::secrets_store::secrets_store;
use crate::config::ProviderConfig;
use greentic_types::messaging::universal_dto::AuthUserRefV1;
use serde_json::Value;
use urlencoding::encode as url_encode;

const DEFAULT_GRAPH_AUTHORITY: &str = "https://login.microsoftonline.com";
const DEFAULT_GRAPH_SCOPE: &str = "https://graph.microsoft.com/.default offline_access openid";
const MS_GRAPH_CLIENT_ID_KEY: &str = "MS_GRAPH_CLIENT_ID";
const MS_GRAPH_CLIENT_SECRET_KEY: &str = "MS_GRAPH_CLIENT_SECRET";
const MS_GRAPH_REFRESH_TOKEN_KEY: &str = "MS_GRAPH_REFRESH_TOKEN";

pub(crate) fn acquire_graph_token(
    cfg: &ProviderConfig,
    user: &AuthUserRefV1,
) -> Result<String, String> {
    let refresh_token = get_secret_any_case(&user.token_key)?;
    let client_id = get_secret_any_case(MS_GRAPH_CLIENT_ID_KEY)?;
    let client_secret = get_secret_any_case(MS_GRAPH_CLIENT_SECRET_KEY).ok();
    let endpoint = graph_token_endpoint(cfg, user)?;
    let scope = cfg.graph_scope.as_deref().unwrap_or(DEFAULT_GRAPH_SCOPE);
    let mut form = format!(
        "client_id={}&grant_type=refresh_token&refresh_token={}&scope={}",
        url_encode(&client_id),
        url_encode(&refresh_token),
        url_encode(scope)
    );
    if let Some(secret) = client_secret {
        form.push_str(&format!("&client_secret={}", url_encode(&secret)));
    }
    request_token(&endpoint, form.as_bytes())
}

/// Acquire token using config values (populated by config_from_secrets).
/// Uses config fields first, falls back to secrets store.
/// Tries refresh_token grant first, falls back to client_credentials.
pub(crate) fn acquire_graph_token_from_store(cfg: &ProviderConfig) -> Result<String, String> {
    let client_id = cfg
        .graph_client_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| get_secret_any_case(MS_GRAPH_CLIENT_ID_KEY).ok())
        .ok_or_else(|| {
            "missing graph_client_id (seed 'ms_graph_client_id' secret)".to_string()
        })?;
    let client_secret = cfg
        .graph_client_secret
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| get_secret_any_case(MS_GRAPH_CLIENT_SECRET_KEY).ok());
    let tenant_id = cfg
        .graph_tenant_id
        .as_deref()
        .ok_or_else(|| "missing graph_tenant_id in config".to_string())?;
    let authority = cfg
        .graph_authority
        .as_deref()
        .unwrap_or(DEFAULT_GRAPH_AUTHORITY);
    let endpoint = format!(
        "{}/{}/oauth2/v2.0/token",
        authority.trim_end_matches('/'),
        tenant_id.trim_matches('/')
    );
    let scope = cfg.graph_scope.as_deref().unwrap_or(DEFAULT_GRAPH_SCOPE);

    // Try refresh_token grant first (from config, then secrets store)
    let refresh_token = cfg
        .graph_refresh_token
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| get_secret_any_case(MS_GRAPH_REFRESH_TOKEN_KEY).ok());
    if let Some(refresh_token) = refresh_token {
        let mut form = format!(
            "client_id={}&grant_type=refresh_token&refresh_token={}&scope={}",
            url_encode(&client_id),
            url_encode(&refresh_token),
            url_encode(scope)
        );
        if let Some(ref secret) = client_secret {
            form.push_str(&format!("&client_secret={}", url_encode(secret)));
        }
        if let Ok(token) = request_token(&endpoint, form.as_bytes()) {
            return Ok(token);
        }
    }

    // Fall back to client_credentials grant (app-only token)
    let secret =
        client_secret.ok_or_else(|| "no refresh_token or client_secret available".to_string())?;
    let cc_scope = "https://graph.microsoft.com/.default";
    let form = format!(
        "client_id={}&client_secret={}&grant_type=client_credentials&scope={}",
        url_encode(&client_id),
        url_encode(&secret),
        url_encode(cc_scope)
    );
    request_token(&endpoint, form.as_bytes())
}

fn graph_token_endpoint(cfg: &ProviderConfig, user: &AuthUserRefV1) -> Result<String, String> {
    if let Some(endpoint) = cfg.graph_token_endpoint.as_ref() {
        return Ok(endpoint.clone());
    }
    let tenant = user
        .tenant_id
        .as_deref()
        .or(cfg.graph_tenant_id.as_deref())
        .ok_or_else(|| "missing Graph tenant id".to_string())?;
    let authority = cfg
        .graph_authority
        .as_deref()
        .unwrap_or(DEFAULT_GRAPH_AUTHORITY);
    Ok(format!(
        "{}/{}/oauth2/v2.0/token",
        authority.trim_end_matches('/'),
        tenant.trim_matches('/')
    ))
}

pub(crate) fn get_secret_any_case(key: &str) -> Result<String, String> {
    get_secret(key).or_else(|_| get_secret(&key.to_ascii_lowercase()))
}

fn get_secret(key: &str) -> Result<String, String> {
    match secrets_store::get(key) {
        Ok(Some(bytes)) => String::from_utf8(bytes).map_err(|_| format!("secret {key} not utf-8")),
        Ok(None) => Err(format!("missing secret: {key}")),
        Err(err) => Err(format!("secret store error: {err:?}")),
    }
}

fn request_token(url: &str, body: &[u8]) -> Result<String, String> {
    let request = client::Request {
        method: "POST".into(),
        url: url.to_string(),
        headers: vec![(
            "Content-Type".into(),
            "application/x-www-form-urlencoded".into(),
        )],
        body: Some(body.to_vec()),
    };
    let resp = client::send(&request, None, None)
        .map_err(|e| format!("token exchange error: {}", e.message))?;
    if resp.status < 200 || resp.status >= 300 {
        let err_body = resp
            .body
            .as_deref()
            .and_then(|b| std::str::from_utf8(b).ok())
            .unwrap_or("");
        return Err(format!(
            "token endpoint returned status {} body={}",
            resp.status, err_body
        ));
    }
    let body = resp.body.unwrap_or_default();
    let parsed: Value =
        serde_json::from_slice(&body).map_err(|e| format!("invalid token response: {e}"))?;
    parsed
        .get("access_token")
        .and_then(Value::as_str)
        .map(|token| token.to_string())
        .ok_or_else(|| "token response missing access_token".to_string())
}
