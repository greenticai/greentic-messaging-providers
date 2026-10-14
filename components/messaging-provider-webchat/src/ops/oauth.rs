//! OAuth HTTP endpoints exposed by the webchat provider.
//!
//! - `GET  /auth/config`         — returns public-safe OAuth configuration for
//!   the frontend SPA. Never exposes `client_secret`.
//! - `POST /oauth/token-exchange` — performs the authorization-code exchange
//!   server-side so that `client_secret` stays out of the browser.
//!
//! Both endpoints read provider configuration from the `ConfigAwareSecretStore`
//! (which prefers the per-request injected config over the host secrets-store
//! interface).

use base64::{Engine as _, engine::general_purpose};
use greentic_types::messaging::universal_dto::{Header, HttpInV1, HttpOutV1};
use provider_common::http_compat::{http_out_error, http_out_v1_bytes};
use serde_json::{Value, json};

use crate::bindings::greentic::http::http_client as client;
use crate::directline::ConfigAwareSecretStore;
use crate::directline::store::SecretStore as _;
use webchat_directline_core::directline::microsoft_authority;

/// Return OAuth configuration as JSON for the frontend SPA.
/// Reads oauth_* fields from secrets store (host interface).
/// Never exposes client_secret — only public-safe fields are returned.
pub(super) fn handle_auth_config(request: &HttpInV1) -> Vec<u8> {
    let secrets = ConfigAwareSecretStore::new(request.config.clone());
    let read_secret = |key: &str| -> Option<String> {
        secrets
            .get(key)
            .ok()
            .flatten()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };

    let enabled = read_secret("oauth_enabled")
        .map(|v| v == "true")
        .unwrap_or(false);

    let body = if enabled {
        // Try pre-composed oauth_providers first, then build from individual fields
        let providers = if let Some(providers_json) = read_secret("oauth_providers") {
            let mut parsed: Value = serde_json::from_str(&providers_json).unwrap_or(json!([]));
            // Strip client_secret — never expose secrets to the frontend
            if let Some(arr) = parsed.as_array_mut() {
                for p in arr.iter_mut() {
                    if let Some(obj) = p.as_object_mut() {
                        obj.remove("client_secret");
                    }
                }
            }
            parsed
        } else {
            let mut list = Vec::new();
            if read_secret("oauth_enable_greentic").as_deref() == Some("true") {
                let client_id = read_secret("oauth_greentic_client_id")
                    .unwrap_or_else(|| "webchat-gui".to_string());
                let mut entry = json!({
                    "id": "greentic", "label": "Greentic SSO", "type": "greentic",
                    "client_id": client_id,
                    "scopes": "openid profile email greentic.webchat"
                });
                if let Some(issuer) = read_secret("oauth_greentic_issuer") {
                    entry["issuer"] = Value::String(issuer);
                }
                list.push(entry);
            }
            if read_secret("oauth_enable_google").as_deref() == Some("true")
                && let Some(client_id) = read_secret("oauth_google_client_id")
            {
                list.push(json!({
                    "id": "google", "label": "Google",
                    "auth_url": "https://accounts.google.com/o/oauth2/v2/auth",
                    "token_url": "https://oauth2.googleapis.com/token",
                    "client_id": client_id, "scopes": "openid profile email"
                }));
            }
            if read_secret("oauth_enable_microsoft").as_deref() == Some("true")
                && let Some(client_id) = read_secret("oauth_microsoft_client_id")
                && let Ok((auth_url, token_url)) = microsoft_authority::endpoints(
                    read_secret("oauth_microsoft_authority").as_deref(),
                )
            {
                list.push(json!({
                    "id": "microsoft", "label": "Microsoft",
                    "auth_url": auth_url,
                    "token_url": token_url,
                    "client_id": client_id, "scopes": "openid profile email"
                }));
            }
            if read_secret("oauth_enable_github").as_deref() == Some("true")
                && let Some(client_id) = read_secret("oauth_github_client_id")
            {
                list.push(json!({
                    "id": "github", "label": "GitHub",
                    "auth_url": "https://github.com/login/oauth/authorize",
                    "token_url": "https://github.com/login/oauth/access_token",
                    "client_id": client_id, "scopes": "read:user user:email"
                }));
            }
            if read_secret("oauth_enable_custom").as_deref() == Some("true")
                && let Some(client_id) = read_secret("oauth_custom_client_id")
            {
                let label = read_secret("oauth_custom_label").unwrap_or_else(|| "SSO".to_string());
                let auth_url = read_secret("oauth_custom_auth_url").unwrap_or_default();
                let token_url = read_secret("oauth_custom_token_url").unwrap_or_default();
                let scopes = read_secret("oauth_custom_scopes")
                    .unwrap_or_else(|| "openid profile email".to_string());
                list.push(json!({
                    "id": "custom", "label": label,
                    "auth_url": auth_url, "token_url": token_url,
                    "client_id": client_id, "scopes": scopes
                }));
            }
            Value::Array(list)
        };
        json!({
            "enabled": true,
            "providers": providers,
        })
    } else {
        json!({ "enabled": false })
    };

    let body_bytes = serde_json::to_vec(&body).unwrap_or_default();
    let out = HttpOutV1 {
        status: 200,
        headers: vec![Header {
            name: "Content-Type".to_string(),
            value: "application/json".to_string(),
        }],
        body_b64: general_purpose::STANDARD.encode(&body_bytes),
        events: Vec::new(),
    };
    http_out_v1_bytes(&out)
}

/// Server-side OAuth token exchange.
/// Receives `provider_id`, `code`, `redirect_uri`, `client_id`, `code_verifier` from the frontend,
/// looks up `client_secret` from secrets store, and POSTs to the provider's token endpoint.
pub(super) fn handle_oauth_token_exchange(request: &HttpInV1) -> Vec<u8> {
    let body_bytes = general_purpose::STANDARD
        .decode(&request.body_b64)
        .unwrap_or_default();
    let payload: Value = match serde_json::from_slice(&body_bytes) {
        Ok(v) => v,
        Err(_) => return http_out_error(400, "invalid JSON body"),
    };

    let provider_id = payload["provider_id"].as_str().unwrap_or("");
    let token_url = payload["token_url"].as_str().unwrap_or("");
    let code = payload["code"].as_str().unwrap_or("");
    let redirect_uri = payload["redirect_uri"].as_str().unwrap_or("");
    let client_id = payload["client_id"].as_str().unwrap_or("");
    let code_verifier = payload["code_verifier"].as_str().unwrap_or("");

    if token_url.is_empty() || code.is_empty() || client_id.is_empty() {
        return http_out_error(400, "missing required fields: token_url, code, client_id");
    }

    // Look up client_secret from secrets store based on provider_id
    let secrets = ConfigAwareSecretStore::new(request.config.clone());
    let read_secret = |key: &str| -> Option<String> {
        secrets
            .get(key)
            .ok()
            .flatten()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };

    // Look up client_secret: try individual key first, then pre-composed oauth_providers
    let secret_key = match provider_id {
        "google" => "oauth_google_client_secret",
        "microsoft" => "oauth_microsoft_client_secret",
        "github" => "oauth_github_client_secret",
        "custom" => "oauth_custom_client_secret",
        _ => "",
    };

    let client_secret = if !secret_key.is_empty() {
        read_secret(secret_key)
    } else {
        None
    }
    .or_else(|| {
        // Fallback: extract from pre-composed oauth_providers JSON
        read_secret("oauth_providers").and_then(|json| {
            serde_json::from_str::<Vec<Value>>(&json)
                .unwrap_or_default()
                .iter()
                .find(|p| p["id"].as_str() == Some(provider_id))
                .and_then(|p| p["client_secret"].as_str())
                .map(|s| s.to_string())
        })
    })
    .unwrap_or_default();

    // Build form-encoded body for token exchange
    let mut form_parts = vec![
        format!("grant_type=authorization_code"),
        format!("code={}", urlencoding::encode(code)),
        format!("redirect_uri={}", urlencoding::encode(redirect_uri)),
        format!("client_id={}", urlencoding::encode(client_id)),
    ];
    if !client_secret.is_empty() {
        form_parts.push(format!(
            "client_secret={}",
            urlencoding::encode(&client_secret)
        ));
    }
    if !code_verifier.is_empty() {
        form_parts.push(format!(
            "code_verifier={}",
            urlencoding::encode(code_verifier)
        ));
    }
    let form_body = form_parts.join("&");

    // Call the OAuth provider's token endpoint
    let http_req = client::Request {
        method: "POST".into(),
        url: token_url.to_string(),
        headers: vec![(
            "Content-Type".into(),
            "application/x-www-form-urlencoded".into(),
        )],
        body: Some(form_body.into_bytes()),
    };

    let token_response = match client::send(&http_req, None, None) {
        Ok(resp) => resp,
        Err(err) => {
            return http_out_error(
                502,
                &format!("token endpoint request failed: {}", err.message),
            );
        }
    };

    // Forward the token endpoint response to the frontend
    let response_body = token_response.body.unwrap_or_default();
    let out = HttpOutV1 {
        status: token_response.status,
        headers: vec![
            Header {
                name: "Content-Type".to_string(),
                value: "application/json".to_string(),
            },
            Header {
                name: "Cache-Control".to_string(),
                value: "no-store".to_string(),
            },
        ],
        body_b64: general_purpose::STANDARD.encode(&response_body),
        events: Vec::new(),
    };
    http_out_v1_bytes(&out)
}
