// The verified caller block a Direct Line turn hands the runtime.
//
// greentic-runner reads `extensions.caller` off the envelope this provider
// emits (greentic-runner#762) and forwards it to tools as `_caller`
// (greentic-runner#760). Its shape is the runner's `VerifiedCaller`:
// `{ user_verified, sub?, groups?, team?, role? }`.
//
// ## Where every field comes from
//
// Only from a Direct Line token whose signature has already verified against
// this provider's signing key — never from the activity body, `from.id`, or
// Action.Submit data, all of which the client writes.
//
// - `user_verified` — the token's `verified` claim.
// - `sub` — the token's `sub`.
// - `team` — the token's signed `ctx.team`.
// - `groups`, `role` — extra claims on the token (greentic-start#584 carries
//   an identity provider's claims through its re-mint). Emitted only when the
//   token is `verified`: a consumer must gate on `user_verified` anyway, and
//   an unverified token has no business asserting authorization claims.
//
// ## Degrading
//
// Every field but `user_verified` is optional. A claim that is absent, or
// present in a shape the runner could not decode (`groups` that is not an
// array of strings), is OMITTED rather than coerced: the runner decodes the
// whole block or none of it, so one malformed claim would otherwise turn a
// verified caller anonymous — and a partially-kept `groups` array would read
// as a complete one.
//
// ## Transport inside the provider
//
// The Direct Line handler verifies the token; the ingest step that builds the
// envelope only sees the handler's response. The block therefore crosses that
// boundary as the [`CALLER_HEADER`] response header (base64url JSON, so
// non-ASCII group names survive), which the ingest step consumes and removes
// before the response leaves the provider.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use greentic_types::messaging::universal_dto::Header;
use serde_json::{Map, Value};

use super::jwt::TokenClaims;

/// Internal response header carrying the caller block from the Direct Line
/// handler to the ingest step.
pub const CALLER_HEADER: &str = "X-Greentic-Caller";

/// The runner-side envelope extension key (`caller_identity::CALLER_EXT_KEY`).
pub const CALLER_EXT_KEY: &str = "caller";

/// Build the caller block from verified token claims.
pub fn caller_block(claims: &TokenClaims) -> Value {
    let mut block = Map::new();
    block.insert("user_verified".into(), Value::Bool(claims.verified));
    if let Some(sub) = non_empty(&claims.sub) {
        block.insert("sub".into(), Value::String(sub.to_string()));
    }
    if let Some(team) = claims.ctx.team.as_deref().and_then(non_empty) {
        block.insert("team".into(), Value::String(team.to_string()));
    }
    if claims.verified {
        if let Some(groups) = string_array(claims.extra.get("groups")) {
            block.insert("groups".into(), groups);
        }
        if let Some(role) = claims
            .extra
            .get("role")
            .and_then(Value::as_str)
            .and_then(non_empty)
        {
            block.insert("role".into(), Value::String(role.to_string()));
        }
    }
    Value::Object(block)
}

/// The [`CALLER_HEADER`] carrying [`caller_block`] for `claims`.
pub fn caller_header(claims: &TokenClaims) -> Header {
    let json = serde_json::to_vec(&caller_block(claims)).unwrap_or_default();
    Header {
        name: CALLER_HEADER.to_string(),
        value: URL_SAFE_NO_PAD.encode(json),
    }
}

/// Decode a [`CALLER_HEADER`] value. `None` for anything that is not a
/// base64url-encoded JSON object carrying a boolean `user_verified`.
pub fn decode_caller_header(value: &str) -> Option<Value> {
    let bytes = URL_SAFE_NO_PAD.decode(value.trim()).ok()?;
    let block: Value = serde_json::from_slice(&bytes).ok()?;
    block.get("user_verified")?.as_bool()?;
    Some(block)
}

fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

/// `groups` only when it is a non-empty array made entirely of strings.
fn string_array(value: Option<&Value>) -> Option<Value> {
    let items = value?.as_array()?;
    if items.is_empty() || !items.iter().all(Value::is_string) {
        return None;
    }
    Some(Value::Array(items.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::directline::jwt::DirectLineContext;
    use serde_json::json;

    fn claims(verified: bool, team: Option<&str>, extra: Value) -> TokenClaims {
        TokenClaims {
            iss: "greentic.webchat".into(),
            aud: "directline".into(),
            sub: "alice".into(),
            iat: 0,
            nbf: 0,
            exp: i64::MAX,
            ctx: DirectLineContext {
                env: "default".into(),
                tenant: "acme".into(),
                team: team.map(str::to_string),
            },
            conv: Some("conv-1".into()),
            verified,
            extra: extra.as_object().cloned().unwrap_or_default(),
        }
    }

    #[test]
    fn a_verified_token_yields_sub_team_groups_and_role() {
        let block = caller_block(&claims(
            true,
            Some("ops"),
            json!({"groups": ["hr", "ops"], "role": "manager", "email": "a@x"}),
        ));
        assert_eq!(
            block,
            json!({
                "user_verified": true, "sub": "alice", "team": "ops",
                "groups": ["hr", "ops"], "role": "manager",
            })
        );
    }

    #[test]
    fn a_token_without_extra_claims_degrades_to_sub_and_team() {
        let block = caller_block(&claims(true, None, json!({})));
        assert_eq!(block, json!({"user_verified": true, "sub": "alice"}));
    }

    #[test]
    fn an_unverified_token_never_asserts_groups_or_role() {
        let block = caller_block(&claims(
            false,
            Some("ops"),
            json!({"groups": ["admins"], "role": "admin"}),
        ));
        assert_eq!(
            block,
            json!({"user_verified": false, "sub": "alice", "team": "ops"})
        );
    }

    #[test]
    fn malformed_claims_are_omitted_not_coerced() {
        let block = caller_block(&claims(
            true,
            Some("  "),
            json!({"groups": ["hr", 7], "role": ["admin"]}),
        ));
        assert_eq!(block, json!({"user_verified": true, "sub": "alice"}));
        let block = caller_block(&claims(true, None, json!({"groups": "hr"})));
        assert!(block.get("groups").is_none());
    }

    #[test]
    fn the_header_round_trips_non_ascii_groups() {
        let claims = claims(true, None, json!({"groups": ["ventes-équipe"]}));
        let header = caller_header(&claims);
        assert_eq!(header.name, CALLER_HEADER);
        assert!(header.value.is_ascii());
        assert_eq!(
            decode_caller_header(&header.value),
            Some(caller_block(&claims))
        );
    }

    #[test]
    fn undecodable_header_values_are_rejected() {
        assert_eq!(decode_caller_header("not base64!"), None);
        let not_object = URL_SAFE_NO_PAD.encode(b"[1,2]");
        assert_eq!(decode_caller_header(&not_object), None);
        let no_flag = URL_SAFE_NO_PAD.encode(br#"{"sub":"alice"}"#);
        assert_eq!(decode_caller_header(&no_flag), None);
    }
}
