//! Verification of a Microsoft Entra ID token (RS256) for "Sign in with
//! Microsoft", and the configuration deciding whether a verified identity may
//! be minted at all.
//!
//! Pure: no network, no clock. The caller fetches the discovery document and
//! the JWKS and passes them in.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rsa::sha2::{Digest, Sha256};
use rsa::{BigUint, Pkcs1v15Sign, RsaPublicKey};
use serde::Deserialize;
use serde_json::Value;

use super::microsoft_authority::{self, DEFAULT_AUTHORITY};

pub const LOGIN_HOST_PREFIX: &str = "https://login.microsoftonline.com/";
const LEEWAY_SECONDS: i64 = 60;
const MAX_JWKS_KEYS: usize = 32;
const MAX_ALLOWED_TENANTS: usize = 64;
const MAX_NONCE_LEN: usize = 256;
const MAX_DESCRIPTIVE_LEN: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Authority {
    /// A tenant GUID or a tenant domain: every token must come from it.
    Single(String),
    /// `common`, `organizations` or `consumers`: any tenant's user signs in.
    Multi(String),
}

impl Authority {
    pub fn segment(&self) -> &str {
        match self {
            Authority::Single(value) | Authority::Multi(value) => value,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntraConfig {
    pub client_id: String,
    pub authority: Authority,
    pub allowed_tenants: Vec<String>,
}

impl EntraConfig {
    pub fn discovery_url(&self) -> String {
        format!(
            "{LOGIN_HOST_PREFIX}{}/v2.0/.well-known/openid-configuration",
            self.authority.segment()
        )
    }
}

/// The identity-minting configuration, or `None` when a verified Microsoft
/// identity must not be minted: the provider is off, has no client id, has an
/// invalid authority, or is multi-tenant with no tenant allow-list.
pub fn resolve(lookup: impl Fn(&str) -> Option<String>) -> Option<EntraConfig> {
    let master_off = lookup("oauth_enabled").is_some_and(|v| !v.eq_ignore_ascii_case("true"));
    let enabled = lookup("oauth_enable_microsoft").is_some_and(|v| v.eq_ignore_ascii_case("true"));
    if master_off || !enabled {
        return None;
    }
    let client_id = lookup("oauth_microsoft_client_id")?;
    let raw = lookup("oauth_microsoft_authority");
    microsoft_authority::endpoints(raw.as_deref()).ok()?;
    let segment = raw
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(DEFAULT_AUTHORITY)
        .to_ascii_lowercase();
    let allowed_tenants = parse_allowed_tenants(lookup("oauth_microsoft_allowed_tenants"))?;
    let authority = if matches!(segment.as_str(), "common" | "organizations" | "consumers") {
        if allowed_tenants.is_empty() {
            return None;
        }
        Authority::Multi(segment)
    } else {
        Authority::Single(segment)
    };
    Some(EntraConfig {
        client_id,
        authority,
        allowed_tenants,
    })
}

/// Comma/whitespace separated tenant GUIDs. `None` when any entry is not a
/// GUID: a typo must disable minting, never silently shrink the list.
fn parse_allowed_tenants(raw: Option<String>) -> Option<Vec<String>> {
    let mut out = Vec::new();
    for entry in raw
        .as_deref()
        .unwrap_or("")
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|e| !e.is_empty())
    {
        if !is_guid(entry) {
            return None;
        }
        let entry = entry.to_ascii_lowercase();
        if !out.contains(&entry) {
            out.push(entry);
        }
    }
    (out.len() <= MAX_ALLOWED_TENANTS).then_some(out)
}

/// Whether `raw` is a well-formed `oauth_microsoft_allowed_tenants` answer.
pub fn allowed_tenants_valid(raw: &str) -> bool {
    parse_allowed_tenants(Some(raw.to_string())).is_some()
}

pub fn is_guid(value: &str) -> bool {
    let groups: Vec<&str> = value.split('-').collect();
    groups.len() == 5
        && [8, 4, 4, 4, 12]
            .iter()
            .zip(&groups)
            .all(|(len, g)| g.len() == *len && g.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// The tenant id inside `https://login.microsoftonline.com/<tid>/v2.0`.
pub fn tenant_of_issuer(issuer: &str) -> Option<String> {
    let tid = issuer
        .strip_prefix(LOGIN_HOST_PREFIX)?
        .strip_suffix("/v2.0")?;
    is_guid(tid).then(|| tid.to_ascii_lowercase())
}

#[derive(Debug)]
pub struct Discovery {
    pub jwks_uri: String,
    /// Tenant the document describes; `None` for the multi-tenant authorities
    /// (their issuer is a `{tenantid}` template).
    pub tenant: Option<String>,
}

pub fn parse_discovery(doc: &str) -> Result<Discovery, EntraError> {
    let value: Value = serde_json::from_str(doc).map_err(|_| EntraError::Discovery)?;
    let jwks_uri = value
        .get("jwks_uri")
        .and_then(Value::as_str)
        .ok_or(EntraError::Discovery)?;
    let rest = jwks_uri
        .strip_prefix(LOGIN_HOST_PREFIX)
        .ok_or(EntraError::Discovery)?;
    if rest.is_empty() || rest.contains(['?', '#', '@', '\\']) || rest.contains("..") {
        return Err(EntraError::Discovery);
    }
    let tenant = value
        .get("issuer")
        .and_then(Value::as_str)
        .and_then(tenant_of_issuer);
    Ok(Discovery {
        jwks_uri: jwks_uri.to_string(),
        tenant,
    })
}

/// The one tenant a single-tenant authority admits: its GUID, or (for a
/// domain) the tenant its discovery document names. `None` when that cannot
/// be established, which must refuse the login.
pub fn single_tenant(config: &EntraConfig, discovery: &Discovery) -> Option<String> {
    let Authority::Single(segment) = &config.authority else {
        return None;
    };
    if is_guid(segment) {
        return match &discovery.tenant {
            Some(tenant) if tenant != segment => None,
            _ => Some(segment.clone()),
        };
    }
    discovery.tenant.clone()
}

#[derive(Debug, PartialEq, Eq)]
pub enum EntraError {
    Discovery,
    InvalidFormat,
    UnsupportedAlg,
    UnknownKey,
    InvalidSignature,
    IssuerMismatch,
    AudienceMismatch,
    Expired,
    NotYetValid,
    NonceMismatch,
    TenantNotAllowed,
    MissingTenant,
    MissingObjectId,
}

#[derive(Debug, PartialEq, Eq)]
pub struct EntraIdentity {
    /// `ms:<tid>:<oid>`
    pub sub: String,
    pub tid: String,
    pub oid: String,
    pub issuer: String,
    pub email: Option<String>,
    pub name: Option<String>,
}

pub struct Expect<'a> {
    pub config: &'a EntraConfig,
    /// The tenant a single-tenant authority resolved to (its GUID, or the one
    /// its discovery document names for a domain). Ignored for multi-tenant.
    pub single_tenant: Option<&'a str>,
    pub nonce: &'a str,
    pub now: i64,
}

#[derive(Deserialize)]
struct Header {
    alg: String,
    #[serde(default)]
    kid: Option<String>,
}

#[derive(Deserialize)]
struct Claims {
    iss: String,
    aud: Value,
    exp: i64,
    #[serde(default)]
    nbf: Option<i64>,
    #[serde(default)]
    nonce: Option<String>,
    #[serde(default)]
    tid: Option<String>,
    #[serde(default)]
    oid: Option<String>,
    #[serde(default)]
    preferred_username: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize)]
struct Jwk {
    kty: String,
    #[serde(default)]
    kid: Option<String>,
    #[serde(default)]
    n: Option<String>,
    #[serde(default)]
    e: Option<String>,
}

#[derive(Deserialize)]
struct Jwks {
    keys: Vec<Jwk>,
}

fn decode_part<T: for<'de> Deserialize<'de>>(part: &str) -> Result<T, EntraError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(part)
        .map_err(|_| EntraError::InvalidFormat)?;
    serde_json::from_slice(&bytes).map_err(|_| EntraError::InvalidFormat)
}

fn public_key(jwk: &Jwk) -> Option<RsaPublicKey> {
    if jwk.kty != "RSA" {
        return None;
    }
    let n = URL_SAFE_NO_PAD.decode(jwk.n.as_deref()?).ok()?;
    let e = URL_SAFE_NO_PAD.decode(jwk.e.as_deref()?).ok()?;
    RsaPublicKey::new(BigUint::from_bytes_be(&n), BigUint::from_bytes_be(&e)).ok()
}

/// Verifies `token` against `jwks_json` and the expectations. The signature is
/// checked before any claim is trusted.
pub fn verify_id_token(
    token: &str,
    jwks_json: &str,
    expect: &Expect<'_>,
) -> Result<EntraIdentity, EntraError> {
    let mut parts = token.split('.');
    let (Some(header_enc), Some(claims_enc), Some(sig_enc), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(EntraError::InvalidFormat);
    };
    let header: Header = decode_part(header_enc)?;
    if header.alg != "RS256" {
        return Err(EntraError::UnsupportedAlg);
    }
    let jwks: Jwks = serde_json::from_str(jwks_json).map_err(|_| EntraError::UnknownKey)?;
    if jwks.keys.len() > MAX_JWKS_KEYS {
        return Err(EntraError::UnknownKey);
    }
    let signature = URL_SAFE_NO_PAD
        .decode(sig_enc)
        .map_err(|_| EntraError::InvalidFormat)?;
    let digest = Sha256::digest(format!("{header_enc}.{claims_enc}").as_bytes());

    let mut saw_key = false;
    let mut verified = false;
    for jwk in jwks.keys.iter().filter(|k| match (&header.kid, &k.kid) {
        (Some(want), Some(have)) => want == have,
        (Some(_), None) => false,
        (None, _) => true,
    }) {
        let Some(key) = public_key(jwk) else { continue };
        saw_key = true;
        if key
            .verify(Pkcs1v15Sign::new::<Sha256>(), &digest, &signature)
            .is_ok()
        {
            verified = true;
            break;
        }
    }
    if !verified {
        return Err(if saw_key {
            EntraError::InvalidSignature
        } else {
            EntraError::UnknownKey
        });
    }

    let claims: Claims = decode_part(claims_enc)?;
    check_claims(claims, expect)
}

fn check_claims(claims: Claims, expect: &Expect<'_>) -> Result<EntraIdentity, EntraError> {
    let tid = claims
        .tid
        .as_deref()
        .filter(|t| is_guid(t))
        .ok_or(EntraError::MissingTenant)?
        .to_ascii_lowercase();
    if claims.iss != format!("{LOGIN_HOST_PREFIX}{tid}/v2.0") {
        return Err(EntraError::IssuerMismatch);
    }
    let aud_ok = match &claims.aud {
        Value::String(aud) => aud == &expect.config.client_id,
        _ => false,
    };
    if !aud_ok {
        return Err(EntraError::AudienceMismatch);
    }
    if expect.now - LEEWAY_SECONDS >= claims.exp {
        return Err(EntraError::Expired);
    }
    if claims
        .nbf
        .is_some_and(|nbf| expect.now + LEEWAY_SECONDS < nbf)
    {
        return Err(EntraError::NotYetValid);
    }
    let nonce_ok = !expect.nonce.is_empty()
        && expect.nonce.len() <= MAX_NONCE_LEN
        && claims.nonce.as_deref() == Some(expect.nonce);
    if !nonce_ok {
        return Err(EntraError::NonceMismatch);
    }
    match &expect.config.authority {
        Authority::Single(_) => {
            if expect.single_tenant != Some(tid.as_str()) {
                return Err(EntraError::TenantNotAllowed);
            }
        }
        Authority::Multi(_) => {
            if !expect.config.allowed_tenants.contains(&tid) {
                return Err(EntraError::TenantNotAllowed);
            }
        }
    }
    let oid = claims
        .oid
        .as_deref()
        .filter(|o| is_guid(o))
        .ok_or(EntraError::MissingObjectId)?
        .to_ascii_lowercase();
    Ok(EntraIdentity {
        sub: format!("ms:{tid}:{oid}"),
        issuer: claims.iss,
        email: descriptive(claims.preferred_username).or_else(|| descriptive(claims.email)),
        name: descriptive(claims.name),
        tid,
        oid,
    })
}

/// Descriptive only (never an authorization input): control characters
/// removed, bounded length.
fn descriptive(value: Option<String>) -> Option<String> {
    let cleaned: String = value?
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_DESCRIPTIVE_LEN)
        .collect();
    let trimmed = cleaned.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

#[cfg(test)]
#[path = "oidc_entra_tests.rs"]
mod tests;
