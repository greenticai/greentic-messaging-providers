//! Microsoft Graph authentication and legacy Bot Framework JWT helpers.
//!
//! Handles:
//! - Graph token acquisition for outbound messages
//! - JWT validation for inbound webhooks (Phase 1: decode-only)

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use urlencoding::encode as url_encode;

use crate::bindings::greentic::http::http_client as client;
use crate::config::{ProviderConfig, get_secret, get_secret_any_case};
use crate::{
    DEFAULT_BOT_APP_PASSWORD_KEY, DEFAULT_BOT_TOKEN_ENDPOINT, DEFAULT_BOT_TOKEN_SCOPE,
    DEFAULT_GRAPH_ACCESS_TOKEN_KEY, DEFAULT_GRAPH_CLIENT_SECRET_KEY,
    DEFAULT_GRAPH_REFRESH_TOKEN_KEY,
};

/// Standard Bot Framework JWT clock skew tolerance (seconds).
pub(crate) const CLOCK_SKEW_SECS: u64 = 300;

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

pub(crate) fn acquire_graph_token(cfg: &ProviderConfig) -> Result<String, String> {
    if let Some(token) = cfg
        .access_token
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| get_secret_any_case(DEFAULT_GRAPH_ACCESS_TOKEN_KEY).ok())
    {
        crate::wlog("graph auth: using stored access token (no refresh needed)");
        return Ok(token);
    }

    acquire_graph_token_from_refresh(cfg)
}

pub(crate) fn acquire_graph_token_from_refresh(cfg: &ProviderConfig) -> Result<String, String> {
    let refresh_token = cfg
        .refresh_token
        .clone()
        .or_else(|| get_secret_any_case(DEFAULT_GRAPH_REFRESH_TOKEN_KEY).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            crate::wlog(
                "graph auth: FAILED — no MS_GRAPH_REFRESH_TOKEN or MS_GRAPH_ACCESS_TOKEN available",
            );
            "MS_GRAPH_REFRESH_TOKEN or MS_GRAPH_ACCESS_TOKEN is required for Graph auth".to_string()
        })?;

    let token_url = format!(
        "{}/{}/oauth2/v2.0/token",
        cfg.auth_base_url.trim_end_matches('/'),
        cfg.tenant_id
    );
    let mut form = graph_refresh_token_form(cfg, &refresh_token);
    let client_secret = cfg
        .client_secret
        .clone()
        .or_else(|| get_secret_any_case(DEFAULT_GRAPH_CLIENT_SECRET_KEY).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let client_secret_present = client_secret.is_some();
    if let Some(secret) = client_secret {
        form.push_str("&client_secret=");
        form.push_str(&url_encode(&secret));
    }

    crate::wlog(&format!(
        "graph auth: refreshing access token via {token_url} (client_secret={})",
        if client_secret_present {
            "present"
        } else {
            "absent"
        }
    ));
    send_token_request(&token_url, &form)
}

pub(crate) fn graph_refresh_token_form(cfg: &ProviderConfig, refresh_token: &str) -> String {
    format!(
        "grant_type=refresh_token&client_id={}&refresh_token={}&scope={}",
        url_encode(&cfg.client_id),
        url_encode(refresh_token),
        url_encode(&cfg.token_scope)
    )
}

pub(crate) fn acquire_bot_token(cfg: &ProviderConfig) -> Result<String, String> {
    let app_id = cfg
        .ms_bot_app_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            crate::wlog("bot auth: FAILED — ms_bot_app_id is not configured");
            "ms_bot_app_id is required".to_string()
        })?;

    let password = cfg
        .ms_bot_app_password
        .clone()
        .or_else(|| get_secret(DEFAULT_BOT_APP_PASSWORD_KEY).ok())
        .ok_or_else(|| {
            crate::wlog("bot auth: FAILED — ms_bot_app_password not in config or secret store");
            "ms_bot_app_password is required (config or secret store)".to_string()
        })?;

    let form = format!(
        "grant_type=client_credentials&client_id={}&client_secret={}&scope={}",
        url_encode(app_id),
        url_encode(&password),
        url_encode(DEFAULT_BOT_TOKEN_SCOPE)
    );

    crate::wlog(&format!(
        "bot auth: acquiring Bot Framework token for app {app_id}"
    ));
    send_token_request(DEFAULT_BOT_TOKEN_ENDPOINT, &form)
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

    let resp = client::send(&request, None, None).map_err(|e| {
        crate::wlog(&format!(
            "token endpoint {url} transport error: {}",
            e.message
        ));
        format!("transport error: {}", e.message)
    })?;

    if resp.status < 200 || resp.status >= 300 {
        let err_body = resp
            .body
            .as_ref()
            .and_then(|b| String::from_utf8(b.clone()).ok())
            .unwrap_or_default();
        crate::wlog(&format!(
            "token endpoint {url} rejected with status {}: {}",
            resp.status, err_body
        ));
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

/// JWT header (only the fields we inspect).
#[derive(Debug, Deserialize)]
struct BotHeader {
    alg: Option<String>,
}

/// Validates a JWT token from Bot Framework webhooks.
///
/// Phase 1: claim validation + alg:none rejection (no cryptographic verify).
/// - Rejects `alg:none` and empty/missing algorithm
/// - Rejects tokens with an empty signature segment
/// - Validates audience matches Bot App ID
/// - Requires and validates expiration time
/// - Validates issuer is in the allowed list
///
/// Not an authenticity proof: greentic-start verifies the signature (`inbound_verify`).
pub(crate) fn validate_jwt(token: &str, app_id: &str) -> Result<BotClaims, String> {
    let header = decode_jwt_header(token)?;
    let alg = header.alg.as_deref().unwrap_or("").trim();
    if alg.eq_ignore_ascii_case("none") || alg.is_empty() {
        return Err("rejected: alg:none / unsigned JWT".to_string());
    }

    let claims = decode_jwt_claims(token)?;

    // Validate audience
    let aud = claims.aud.as_deref().unwrap_or_default();
    if aud != app_id {
        return Err(format!(
            "invalid audience: expected {}, got {}",
            app_id, aud
        ));
    }

    // Validate expiration (mandatory, with clock-skew tolerance)
    let exp = claims.exp.ok_or("missing exp claim")?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if exp + CLOCK_SKEW_SECS < now {
        return Err("token expired".to_string());
    }

    // Validate issuer
    let iss = claims.iss.as_deref().unwrap_or_default();
    if !VALID_ISSUERS.contains(&iss) {
        return Err(format!("invalid issuer: {}", iss));
    }

    Ok(claims)
}

/// Decodes a base64url segment (with padding tolerance).
fn url_safe_decode(segment: &str) -> Result<Vec<u8>, base64::DecodeError> {
    URL_SAFE_NO_PAD.decode(segment).or_else(|_| {
        let padded = match segment.len() % 4 {
            2 => format!("{}==", segment),
            3 => format!("{}=", segment),
            _ => segment.to_string(),
        };
        URL_SAFE_NO_PAD.decode(&padded)
    })
}

/// Splits `header.payload.signature` and rejects structurally invalid tokens.
fn split_jwt_segments(token: &str) -> Result<[&str; 3], String> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err("invalid JWT format: expected 3 parts".to_string());
    }
    if parts[2].is_empty() {
        return Err("missing JWT signature".to_string());
    }
    Ok([parts[0], parts[1], parts[2]])
}

/// Parses the JWT header and rejects structurally invalid tokens.
fn decode_jwt_header(token: &str) -> Result<BotHeader, String> {
    let [header, _, _] = split_jwt_segments(token)?;
    let header_bytes =
        url_safe_decode(header).map_err(|e| format!("failed to decode JWT header: {e}"))?;
    serde_json::from_slice(&header_bytes).map_err(|e| format!("failed to parse JWT header: {e}"))
}

/// Decodes JWT payload without signature verification.
///
/// JWT format: header.payload.signature (base64url encoded)
fn decode_jwt_claims(token: &str) -> Result<BotClaims, String> {
    let [_, payload, _] = split_jwt_segments(token)?;
    let payload_bytes =
        url_safe_decode(payload).map_err(|e| format!("failed to decode JWT payload: {e}"))?;
    serde_json::from_slice(&payload_bytes).map_err(|e| format!("failed to parse JWT claims: {e}"))
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

/// Builds a token with RS256 header and a placeholder signature.
/// Signature is not cryptographically valid but satisfies structural checks.
#[cfg(test)]
pub(crate) fn fake_signed_token(claims: Value) -> String {
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
    let payload =
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).expect("Value always serializes"));
    format!("{header}.{payload}.placeholder-sig")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
        assert!(VALID_ISSUERS.iter().all(|iss| iss.starts_with("https://")));
    }

    #[test]
    fn validate_jwt_accepts_expected_botframework_claims() {
        let token = fake_signed_token(json!({
            "iss": "https://api.botframework.com",
            "aud": "bot-app-id",
            "exp": 4_102_444_800_u64,
            "iat": 1_u64,
            "serviceurl": "https://smba.trafficmanager.net/amer/"
        }));

        let claims = validate_jwt(&token, "bot-app-id").expect("valid claims");

        assert_eq!(claims.aud.as_deref(), Some("bot-app-id"));
        assert_eq!(
            claims.service_url.as_deref(),
            Some("https://smba.trafficmanager.net/amer/")
        );
    }

    #[test]
    fn validate_jwt_does_not_verify_the_signature() {
        let claims = json!({
            "iss": "https://api.botframework.com",
            "aud": "bot-app-id",
            "exp": 4_102_444_800_u64
        });
        let token = fake_signed_token(claims);
        let (unsigned, _) = token.rsplit_once('.').expect("three segments");
        let forged = format!("{unsigned}.AAAA");
        assert!(validate_jwt(&forged, "bot-app-id").is_ok());
    }

    #[test]
    fn validate_jwt_rejects_wrong_audience_expired_and_issuer() {
        let valid_exp = 4_102_444_800_u64;
        let wrong_audience = fake_signed_token(json!({
            "iss": "https://api.botframework.com",
            "aud": "other",
            "exp": valid_exp
        }));
        assert!(
            validate_jwt(&wrong_audience, "bot-app-id")
                .expect_err("audience")
                .contains("invalid audience")
        );

        let expired = fake_signed_token(json!({
            "iss": "https://api.botframework.com",
            "aud": "bot-app-id",
            "exp": 1_u64
        }));
        assert_eq!(
            validate_jwt(&expired, "bot-app-id").expect_err("expired"),
            "token expired"
        );

        let bad_issuer = fake_signed_token(json!({
            "iss": "https://evil.example",
            "aud": "bot-app-id",
            "exp": valid_exp
        }));
        assert!(
            validate_jwt(&bad_issuer, "bot-app-id")
                .expect_err("issuer")
                .contains("invalid issuer")
        );
    }

    #[test]
    fn acquire_bot_token_validates_local_config_before_secret_lookup() {
        let cfg = ProviderConfig {
            enabled: true,
            public_base_url: Some("https://example.com".to_string()),
            setup_mode: None,
            tenant_id: "tenant".to_string(),
            client_id: "client".to_string(),
            refresh_token: Some("refresh".to_string()),
            client_secret: None,
            access_token: None,
            graph_base_url: crate::DEFAULT_GRAPH_BASE_URL.to_string(),
            auth_base_url: crate::DEFAULT_AUTH_BASE_URL.to_string(),
            token_scope: crate::DEFAULT_GRAPH_TOKEN_SCOPE.to_string(),
            team_id: None,
            team_name: None,
            channel_id: None,
            channel_name: None,
            desired_channel_name: None,
            chat_id: None,
            user_id: None,
            ms_bot_app_id: Some(" ".to_string()),
            ms_bot_app_password: Some("secret".to_string()),
            bot_display_name: None,
            messaging_endpoint: None,
            default_service_url: None,
            skip_jwt_validation: None,
        };

        assert_eq!(
            acquire_bot_token(&cfg).expect_err("missing app id"),
            "ms_bot_app_id is required"
        );
    }

    #[test]
    fn graph_refresh_token_form_omits_client_secret() {
        let cfg = ProviderConfig {
            enabled: true,
            public_base_url: None,
            setup_mode: None,
            tenant_id: "tenant".to_string(),
            client_id: "client id".to_string(),
            refresh_token: Some("refresh".to_string()),
            client_secret: None,
            access_token: None,
            graph_base_url: crate::DEFAULT_GRAPH_BASE_URL.to_string(),
            auth_base_url: crate::DEFAULT_AUTH_BASE_URL.to_string(),
            token_scope: "scope value".to_string(),
            team_id: None,
            team_name: None,
            channel_id: None,
            channel_name: None,
            desired_channel_name: None,
            chat_id: None,
            user_id: None,
            ms_bot_app_id: None,
            ms_bot_app_password: None,
            bot_display_name: None,
            messaging_endpoint: None,
            default_service_url: None,
            skip_jwt_validation: None,
        };

        let form = graph_refresh_token_form(&cfg, "refresh token");

        assert!(form.contains("grant_type=refresh_token"));
        assert!(form.contains("client_id=client%20id"));
        assert!(form.contains("refresh_token=refresh%20token"));
        assert!(form.contains("scope=scope%20value"));
        assert!(!form.contains("client_secret"));
    }

    #[test]
    fn validate_jwt_rejects_expired_beyond_skew() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let token = fake_signed_token(json!({
            "iss": "https://api.botframework.com",
            "aud": "bot-app-id",
            "exp": now - CLOCK_SKEW_SECS - 1,
        }));
        assert_eq!(
            validate_jwt(&token, "bot-app-id").expect_err("expired"),
            "token expired"
        );
    }

    #[test]
    fn validate_jwt_accepts_expired_within_skew() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let token = fake_signed_token(json!({
            "iss": "https://api.botframework.com",
            "aud": "bot-app-id",
            "exp": now - 60,
        }));
        validate_jwt(&token, "bot-app-id").expect("within skew should pass");
    }

    #[test]
    fn validate_jwt_rejects_malformed_token() {
        assert!(validate_jwt("not.a.jwt", "bot-app-id").is_err());
        assert!(validate_jwt("only-one-part", "bot-app-id").is_err());
    }

    #[test]
    fn validate_jwt_rejects_invalid_issuer() {
        let token = fake_signed_token(json!({
            "iss": "https://evil.example.com",
            "aud": "bot-app-id",
            "exp": 4_102_444_800_u64,
        }));
        let err = validate_jwt(&token, "bot-app-id").expect_err("bad issuer");
        assert!(err.contains("invalid issuer"));
    }

    #[test]
    fn validate_jwt_rejects_wrong_audience() {
        let token = fake_signed_token(json!({
            "iss": "https://api.botframework.com",
            "aud": "wrong-app-id",
            "exp": 4_102_444_800_u64,
        }));
        let err = validate_jwt(&token, "bot-app-id").expect_err("wrong aud");
        assert!(err.contains("invalid audience"));
    }

    #[test]
    fn validate_jwt_rejects_alg_none() {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none","typ":"JWT"}"#);
        let payload = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&json!({
                "iss": "https://api.botframework.com",
                "aud": "bot-app-id",
                "exp": 4_102_444_800_u64,
            }))
            .unwrap(),
        );
        let token = format!("{header}.{payload}.placeholder-sig");
        let err = validate_jwt(&token, "bot-app-id").expect_err("alg:none");
        assert!(err.contains("alg:none"));
    }

    #[test]
    fn validate_jwt_rejects_empty_signature() {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
        let payload = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&json!({
                "iss": "https://api.botframework.com",
                "aud": "bot-app-id",
                "exp": 4_102_444_800_u64,
            }))
            .unwrap(),
        );
        let token = format!("{header}.{payload}.");
        let err = validate_jwt(&token, "bot-app-id").expect_err("empty sig");
        assert!(err.contains("missing JWT signature"));
    }

    #[test]
    fn validate_jwt_rejects_missing_exp() {
        let token = fake_signed_token(json!({
            "iss": "https://api.botframework.com",
            "aud": "bot-app-id",
        }));
        let err = validate_jwt(&token, "bot-app-id").expect_err("no exp");
        assert!(err.contains("missing exp claim"));
    }
}
