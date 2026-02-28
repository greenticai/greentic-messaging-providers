//! Bot Framework authentication module.
//!
//! Handles:
//! - Bot token acquisition for outbound messages (Bot Connector API)
//! - JWT validation for inbound webhooks (Phase 1: decode-only)

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use urlencoding::encode as url_encode;

use crate::bindings::greentic::http::http_client as client;
use crate::config::{ProviderConfig, get_secret};
use crate::{DEFAULT_BOT_APP_PASSWORD_KEY, DEFAULT_BOT_TOKEN_ENDPOINT, DEFAULT_BOT_TOKEN_SCOPE};

/// Valid issuers for Bot Framework JWT tokens.
const VALID_ISSUERS: &[&str] = &[
    "https://api.botframework.com",
    "https://sts.windows.net/d6d49420-f39b-4df7-a1dc-d59a935871db/",
    "https://login.microsoftonline.com/d6d49420-f39b-4df7-a1dc-d59a935871db/v2.0",
    "https://sts.windows.net/f8cdef31-a31e-4b4a-93e4-5f571e91255a/",
    "https://login.microsoftonline.com/f8cdef31-a31e-4b4a-93e4-5f571e91255a/v2.0",
];

/// JWT claims from Bot Framework tokens.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct BotClaims {
    /// Issuer
    pub iss: Option<String>,
    /// Audience (should be Bot App ID)
    pub aud: Option<String>,
    /// Expiration time (Unix timestamp)
    pub exp: Option<u64>,
    /// Issued at (Unix timestamp)
    pub iat: Option<u64>,
    /// Service URL (for Teams activities)
    #[serde(rename = "serviceurl")]
    pub service_url: Option<String>,
}

/// Acquires a Bot Framework token for outbound API calls.
///
/// Uses client credentials flow with the Bot App ID and Password.
/// Token endpoint: `https://login.microsoftonline.com/botframework.com/oauth2/v2.0/token`
/// Scope: `https://api.botframework.com/.default`
pub(crate) fn acquire_bot_token(cfg: &ProviderConfig) -> Result<String, String> {
    let app_id = &cfg.ms_bot_app_id;
    if app_id.trim().is_empty() {
        return Err("ms_bot_app_id is required".to_string());
    }

    let password = cfg
        .ms_bot_app_password
        .clone()
        .or_else(|| get_secret(DEFAULT_BOT_APP_PASSWORD_KEY).ok())
        .ok_or_else(|| "ms_bot_app_password is required (config or secret store)".to_string())?;

    let token_url = DEFAULT_BOT_TOKEN_ENDPOINT;
    let scope = DEFAULT_BOT_TOKEN_SCOPE;

    let form = format!(
        "grant_type=client_credentials&client_id={}&client_secret={}&scope={}",
        url_encode(app_id),
        url_encode(&password),
        url_encode(scope)
    );

    send_token_request(token_url, &form)
}

/// Sends token request to the OAuth endpoint.
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
        let err_body = resp
            .body
            .as_ref()
            .and_then(|b| String::from_utf8(b.clone()).ok())
            .unwrap_or_default();
        return Err(format!(
            "token endpoint returned status {}: {}",
            resp.status, err_body
        ));
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

/// Validates a JWT token from Bot Framework webhooks.
///
/// Phase 1 implementation: Decode-only validation (for dev/testing).
/// - Decodes the JWT payload without cryptographic verification
/// - Validates audience matches Bot App ID
/// - Validates expiration time
/// - Validates issuer is in the allowed list
///
/// Note: Full validation would require fetching Microsoft's public keys from
/// `https://login.botframework.com/v1/.well-known/keys` and verifying the signature.
pub(crate) fn validate_jwt(token: &str, app_id: &str) -> Result<BotClaims, String> {
    let claims = decode_jwt_claims(token)?;

    // Validate audience
    let aud = claims.aud.as_deref().unwrap_or_default();
    if aud != app_id {
        return Err(format!(
            "invalid audience: expected {}, got {}",
            app_id, aud
        ));
    }

    // Validate expiration
    if let Some(exp) = claims.exp {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if exp < now {
            return Err("token expired".to_string());
        }
    }

    // Validate issuer
    let iss = claims.iss.as_deref().unwrap_or_default();
    if !VALID_ISSUERS.iter().any(|valid| *valid == iss) {
        return Err(format!("invalid issuer: {}", iss));
    }

    Ok(claims)
}

/// Decodes JWT payload without signature verification.
///
/// JWT format: header.payload.signature (base64url encoded)
fn decode_jwt_claims(token: &str) -> Result<BotClaims, String> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err("invalid JWT format: expected 3 parts".to_string());
    }

    let payload = parts[1];
    let payload_bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| {
            // Try with padding
            let padded = match payload.len() % 4 {
                2 => format!("{}==", payload),
                3 => format!("{}=", payload),
                _ => payload.to_string(),
            };
            URL_SAFE_NO_PAD.decode(&padded)
        })
        .map_err(|e| format!("failed to decode JWT payload: {e}"))?;

    let claims: BotClaims = serde_json::from_slice(&payload_bytes)
        .map_err(|e| format!("failed to parse JWT claims: {e}"))?;

    Ok(claims)
}

/// Extracts Bearer token from Authorization header.
pub(crate) fn extract_bearer_token(auth_header: &str) -> Option<String> {
    let trimmed = auth_header.trim();
    if trimmed.len() > 7 && trimmed[..7].eq_ignore_ascii_case("bearer ") {
        Some(trimmed[7..].trim().to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_bearer_token_works() {
        assert_eq!(
            extract_bearer_token("Bearer abc123"),
            Some("abc123".to_string())
        );
        assert_eq!(
            extract_bearer_token("bearer abc123"),
            Some("abc123".to_string())
        );
        assert_eq!(
            extract_bearer_token("BEARER abc123"),
            Some("abc123".to_string())
        );
        assert_eq!(extract_bearer_token("Basic abc123"), None);
        assert_eq!(extract_bearer_token(""), None);
    }

    #[test]
    fn decode_jwt_claims_rejects_invalid_format() {
        assert!(decode_jwt_claims("invalid").is_err());
        assert!(decode_jwt_claims("a.b").is_err());
        assert!(decode_jwt_claims("a.b.c.d").is_err());
    }

    #[test]
    fn valid_issuers_are_defined() {
        assert!(!VALID_ISSUERS.is_empty());
        assert!(VALID_ISSUERS.iter().all(|iss| iss.starts_with("https://")));
    }
}
