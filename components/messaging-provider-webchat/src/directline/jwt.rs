use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{Duration, Utc};
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::Sha256;

/// Direct Line token lifetime. Matches the Microsoft Direct Line 3.0 reference
/// value of 24 hours — long enough that a chat session outlives a single token,
/// with `/v3/directline/tokens/refresh` available to extend live conversations.
pub const TTL_SECONDS: i64 = 24 * 60 * 60;
const ISS: &str = "greentic.webchat";
const AUD: &str = "directline";

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DirectLineContext {
    pub env: String,
    pub tenant: String,
    pub team: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TokenClaims {
    pub iss: String,
    pub aud: String,
    pub sub: String,
    pub iat: i64,
    pub nbf: i64,
    pub exp: i64,
    pub ctx: DirectLineContext,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conv: Option<String>,
    /// True when `sub` was bound from a verified OIDC bearer, not an
    /// anonymous/self-declared identity.
    #[serde(default)]
    pub verified: bool,
    /// Every other claim on the token — `groups`, `role`, whatever the minter
    /// put there (greentic-start carries an identity provider's claims through
    /// its re-mint, greentic-start#584).
    ///
    /// Only ever populated by [`verify_token`], i.e. from a payload whose HMAC
    /// signature already verified against this provider's signing key, so these
    /// are exactly as trustworthy as `sub`. Serde routes the named fields above
    /// into their own slots, so a reserved name cannot land here on parse;
    /// [`carried_extra_claims`] strips them anyway before a re-issue signs.
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

/// Claims whose meaning this provider owns. Never copied out of
/// [`TokenClaims::extra`] onto a re-issued token: the named ones are re-stamped
/// by the issuer, and `jti` would replay a single-use id onto a new token.
/// Mirrors greentic-start's `RESERVED_CLAIMS`, plus `verified`, which is a
/// named field here and not there.
const RESERVED_CLAIMS: &[&str] = &[
    "iss", "aud", "sub", "iat", "nbf", "exp", "jti", "ctx", "conv", "verified",
];

/// Upper bound on the serialized size of carried extra claims — the same cap
/// greentic-start applies, for the same reason: the token rides an
/// `Authorization` header on every poll, and proxies refuse headers past
/// ~8 KiB.
pub const MAX_EXTRA_CLAIMS_BYTES: usize = 4096;

/// The extra claims a re-issued token carries: reserved names removed, and the
/// whole set dropped when it exceeds [`MAX_EXTRA_CLAIMS_BYTES`]. Dropped whole,
/// never truncated — a partial `groups` array reads as a complete identity.
pub fn carried_extra_claims(extra: &Map<String, Value>) -> Map<String, Value> {
    let carried: Map<String, Value> = extra
        .iter()
        .filter(|(name, _)| !RESERVED_CLAIMS.contains(&name.as_str()))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let size = serde_json::to_vec(&carried)
        .map(|bytes| bytes.len())
        .unwrap_or(usize::MAX);
    if size > MAX_EXTRA_CLAIMS_BYTES {
        return Map::new();
    }
    carried
}

#[allow(dead_code)]
#[derive(Debug)]
pub enum JwtError {
    InvalidFormat,
    InvalidSignature,
    Expired,
    NotYetValid,
    InvalidKey,
    Json(serde_json::Error),
    Base64(base64::DecodeError),
}

impl From<serde_json::Error> for JwtError {
    fn from(err: serde_json::Error) -> Self {
        JwtError::Json(err)
    }
}

impl From<base64::DecodeError> for JwtError {
    fn from(err: base64::DecodeError) -> Self {
        JwtError::Base64(err)
    }
}

fn encode_segment<T: Serialize>(value: &T) -> Result<String, JwtError> {
    let json = serde_json::to_string(value)?;
    Ok(URL_SAFE_NO_PAD.encode(json.as_bytes()))
}

fn decode_segment<T: for<'de> Deserialize<'de>>(value: &str) -> Result<T, JwtError> {
    let bytes = URL_SAFE_NO_PAD.decode(value)?;
    let decoded = serde_json::from_slice(&bytes)?;
    Ok(decoded)
}

pub fn issue_token(
    secret: &[u8],
    ctx: DirectLineContext,
    sub: &str,
    conv: Option<String>,
    verified: bool,
) -> Result<(String, i64), JwtError> {
    sign_new_token(secret, ctx, sub, conv, verified, Map::new())
}

/// Re-issue `claims` — same `sub`, `ctx`, `verified` and carried extra claims —
/// bound to `conv`, with a fresh lifetime.
///
/// Conversation create, refresh and reconnect all hand the client a NEW token,
/// and the client posts every later activity with that one. Minting it from
/// `sub`/`ctx`/`verified` alone silently dropped any extra claim at the first
/// of those hops, so a caller's groups could never reach an activity.
pub fn reissue_token(
    secret: &[u8],
    claims: &TokenClaims,
    conv: Option<String>,
) -> Result<(String, i64), JwtError> {
    sign_new_token(
        secret,
        claims.ctx.clone(),
        &claims.sub,
        conv,
        claims.verified,
        carried_extra_claims(&claims.extra),
    )
}

fn sign_new_token(
    secret: &[u8],
    ctx: DirectLineContext,
    sub: &str,
    conv: Option<String>,
    verified: bool,
    extra: Map<String, Value>,
) -> Result<(String, i64), JwtError> {
    let now = Utc::now();
    let iat = now.timestamp();
    let exp = (now + Duration::seconds(TTL_SECONDS)).timestamp();
    let claims = TokenClaims {
        iss: ISS.to_string(),
        aud: AUD.to_string(),
        sub: sub.to_string(),
        iat,
        nbf: iat,
        exp,
        ctx,
        conv,
        verified,
        extra,
    };
    let header = serde_json::json!({"alg":"HS256","typ":"JWT"});
    let header_enc = encode_segment(&header)?;
    let payload_enc = encode_segment(&claims)?;
    let mut mac = HmacSha256::new_from_slice(secret).map_err(|_| JwtError::InvalidKey)?;
    mac.update(header_enc.as_bytes());
    mac.update(b".");
    mac.update(payload_enc.as_bytes());
    let signature = mac.finalize().into_bytes();
    let signature_enc = URL_SAFE_NO_PAD.encode(signature);
    let token = format!("{header_enc}.{payload_enc}.{signature_enc}");
    Ok((token, exp))
}

pub fn verify_token(secret: &[u8], token: &str) -> Result<TokenClaims, JwtError> {
    let mut parts = token.split('.');
    let header = parts.next().ok_or(JwtError::InvalidFormat)?;
    let payload = parts.next().ok_or(JwtError::InvalidFormat)?;
    let signature = parts.next().ok_or(JwtError::InvalidFormat)?;
    if parts.next().is_some() {
        return Err(JwtError::InvalidFormat);
    }
    let mut mac = HmacSha256::new_from_slice(secret).map_err(|_| JwtError::InvalidKey)?;
    mac.update(header.as_bytes());
    mac.update(b".");
    mac.update(payload.as_bytes());
    let expected = mac.finalize().into_bytes();
    let decoded_sig = URL_SAFE_NO_PAD.decode(signature)?;
    if expected.as_slice() != decoded_sig {
        return Err(JwtError::InvalidSignature);
    }
    let claims: TokenClaims = decode_segment(payload)?;
    let now = Utc::now().timestamp();
    // Allow a small clock-skew leeway (30 seconds) to avoid NotYetValid
    // errors when the token is created and validated on systems with
    // slightly different clocks (e.g. WSL2 + external device via tunnel).
    let leeway = 30;
    if now + leeway < claims.nbf {
        return Err(JwtError::NotYetValid);
    }
    if now >= claims.exp {
        return Err(JwtError::Expired);
    }
    Ok(claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_ctx() -> DirectLineContext {
        DirectLineContext {
            env: "default".into(),
            tenant: "default".into(),
            team: Some("team-a".into()),
        }
    }

    fn signed_token(signing_key: &[u8], claims: TokenClaims) -> Result<String, JwtError> {
        let header = serde_json::json!({"alg":"HS256","typ":"JWT"});
        let header_enc = encode_segment(&header)?;
        let payload_enc = encode_segment(&claims)?;
        let mut mac = HmacSha256::new_from_slice(signing_key).map_err(|_| JwtError::InvalidKey)?;
        mac.update(header_enc.as_bytes());
        mac.update(b".");
        mac.update(payload_enc.as_bytes());
        let signature_enc = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
        Ok(format!("{header_enc}.{payload_enc}.{signature_enc}"))
    }

    #[test]
    fn token_round_trip() -> Result<(), JwtError> {
        let signing_key = b"test-hmac-key";
        let ctx = sample_ctx();
        let (token, exp) = issue_token(signing_key, ctx.clone(), "user-123", None, false)?;
        assert!(token.split('.').count() == 3);
        assert!(exp > Utc::now().timestamp());
        let claims = verify_token(signing_key, &token)?;
        assert_eq!(claims.sub, "user-123");
        assert_eq!(claims.ctx, ctx);
        assert!(claims.conv.is_none());
        Ok(())
    }

    #[test]
    fn verify_rejects_malformed_tokens() {
        let signing_key = b"test-hmac-key";

        assert!(matches!(
            verify_token(signing_key, "not-enough.parts"),
            Err(JwtError::InvalidFormat)
        ));
        assert!(matches!(
            verify_token(signing_key, "too.many.parts.here"),
            Err(JwtError::InvalidFormat)
        ));
        assert!(matches!(
            verify_token(signing_key, "header.payload.not-base64!"),
            Err(JwtError::Base64(_))
        ));
    }

    #[test]
    fn verify_rejects_wrong_signature() -> Result<(), JwtError> {
        let signing_key = b"test-hmac-key";
        let (token, _) = issue_token(signing_key, sample_ctx(), "user-123", None, false)?;

        assert!(matches!(
            verify_token(b"wrong-hmac-key", &token),
            Err(JwtError::InvalidSignature)
        ));
        Ok(())
    }

    #[test]
    fn verify_rejects_expired_and_not_yet_valid_claims() -> Result<(), JwtError> {
        let signing_key = b"test-hmac-key";
        let now = Utc::now().timestamp();
        let base = TokenClaims {
            iss: ISS.to_string(),
            aud: AUD.to_string(),
            sub: "user-123".to_string(),
            iat: now,
            nbf: now,
            exp: now + TTL_SECONDS,
            ctx: sample_ctx(),
            conv: None,
            verified: false,
            extra: Map::new(),
        };

        let expired = TokenClaims {
            exp: now - 1,
            ..base
        };
        let expired_token = signed_token(signing_key, expired)?;
        assert!(matches!(
            verify_token(signing_key, &expired_token),
            Err(JwtError::Expired)
        ));

        let future = TokenClaims {
            iat: now + 120,
            nbf: now + 120,
            exp: now + TTL_SECONDS,
            ctx: sample_ctx(),
            conv: None,
            iss: ISS.to_string(),
            aud: AUD.to_string(),
            sub: "user-123".to_string(),
            verified: false,
            extra: Map::new(),
        };
        let future_token = signed_token(signing_key, future)?;
        assert!(matches!(
            verify_token(signing_key, &future_token),
            Err(JwtError::NotYetValid)
        ));
        Ok(())
    }

    #[test]
    fn token_with_conv_claim() -> Result<(), JwtError> {
        let signing_key = b"another-test-hmac-key";
        let ctx = DirectLineContext {
            env: "prod".into(),
            tenant: "tenant-a".into(),
            team: None,
        };
        let (token, _) = issue_token(
            signing_key,
            ctx.clone(),
            "user-x",
            Some("conv-99".into()),
            true,
        )?;
        let claims = verify_token(signing_key, &token)?;
        assert_eq!(claims.conv.as_deref(), Some("conv-99"));
        assert_eq!(claims.ctx, ctx);
        Ok(())
    }

    /// Sign an arbitrary payload, so a test can put claims on a token that
    /// `TokenClaims` would not produce itself (an IdP's `groups`, a `jti`).
    fn sign_raw(signing_key: &[u8], payload: &Value) -> String {
        let header_enc =
            encode_segment(&serde_json::json!({"alg":"HS256","typ":"JWT"})).expect("header");
        let payload_enc = encode_segment(payload).expect("payload");
        let mut mac = HmacSha256::new_from_slice(signing_key).expect("key");
        mac.update(header_enc.as_bytes());
        mac.update(b".");
        mac.update(payload_enc.as_bytes());
        let signature_enc = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
        format!("{header_enc}.{payload_enc}.{signature_enc}")
    }

    fn token_with_extra(signing_key: &[u8], extra: Value) -> String {
        let now = Utc::now().timestamp();
        let mut payload = serde_json::json!({
            "iss": ISS, "aud": AUD, "sub": "alice", "iat": now, "nbf": now,
            "exp": now + 600, "ctx": {"env": "default", "tenant": "acme", "team": "ops"},
            "verified": true,
        });
        for (k, v) in extra.as_object().expect("object") {
            payload[k] = v.clone();
        }
        sign_raw(signing_key, &payload)
    }

    #[test]
    fn extra_claims_are_read_from_a_verified_token() {
        let key = b"test-hmac-key";
        let token = token_with_extra(key, serde_json::json!({"groups": ["hr"], "role": "admin"}));
        let claims = verify_token(key, &token).expect("verifies");
        assert_eq!(claims.extra.get("groups"), Some(&serde_json::json!(["hr"])));
        assert_eq!(claims.extra.get("role"), Some(&serde_json::json!("admin")));
        assert!(
            !claims.extra.contains_key("sub"),
            "named claims stay in their slots"
        );
    }

    #[test]
    fn a_token_without_extra_claims_still_verifies_and_issues_none() -> Result<(), JwtError> {
        let key = b"test-hmac-key";
        let (token, _) = issue_token(key, sample_ctx(), "user-123", None, false)?;
        let claims = verify_token(key, &token)?;
        assert!(claims.extra.is_empty());
        let payload: Value = decode_segment(token.split('.').nth(1).expect("payload"))?;
        assert!(
            payload.get("extra").is_none(),
            "an empty map must not serialize"
        );
        Ok(())
    }

    #[test]
    fn reissue_carries_extra_claims_and_binds_the_conversation() -> Result<(), JwtError> {
        let key = b"test-hmac-key";
        let token = token_with_extra(key, serde_json::json!({"groups": ["hr", "ops"]}));
        let claims = verify_token(key, &token)?;
        let (reissued, _) = reissue_token(key, &claims, Some("conv-1".into()))?;
        let again = verify_token(key, &reissued)?;
        assert_eq!(again.sub, "alice");
        assert!(again.verified);
        assert_eq!(again.conv.as_deref(), Some("conv-1"));
        assert_eq!(
            again.extra.get("groups"),
            Some(&serde_json::json!(["hr", "ops"]))
        );
        Ok(())
    }

    #[test]
    fn reissue_never_copies_a_jti() -> Result<(), JwtError> {
        let key = b"test-hmac-key";
        let token = token_with_extra(key, serde_json::json!({"jti": "once", "role": "x"}));
        let claims = verify_token(key, &token)?;
        let (reissued, _) = reissue_token(key, &claims, None)?;
        let payload: Value = decode_segment(reissued.split('.').nth(1).expect("payload"))?;
        assert!(payload.get("jti").is_none());
        assert_eq!(payload.get("role"), Some(&serde_json::json!("x")));
        Ok(())
    }

    #[test]
    fn oversized_extra_claims_are_dropped_whole() {
        let mut extra = Map::new();
        extra.insert(
            "groups".into(),
            serde_json::json!(vec!["g".repeat(64); 100]),
        );
        extra.insert("role".into(), serde_json::json!("admin"));
        assert!(carried_extra_claims(&extra).is_empty());
    }

    #[test]
    fn extra_claims_on_a_forged_signature_are_never_read() {
        let token = token_with_extra(b"attacker-key", serde_json::json!({"groups": ["admins"]}));
        assert!(matches!(
            verify_token(b"test-hmac-key", &token),
            Err(JwtError::InvalidSignature)
        ));
    }
}
