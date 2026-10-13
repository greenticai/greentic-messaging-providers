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
// - `team`, `teams` — ONLY the verified `team_slugs` claim (see below). Never
//   `ctx.team`: that is parsed from the request's `?team=` query parameter,
//   which the client controls, so it is routing context and not identity.
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
    if claims.verified {
        // The team grant comes only from the verified `team_slugs` claim of a
        // token whose signature and issuer already checked out. An unverified
        // token, or one with no valid slug, stays team-less (anonymous).
        let teams = verified_teams(claims.extra.get(super::jwt::TEAMS_CLAIM));
        if let Some(first) = teams.first() {
            block.insert("team".into(), Value::String(first.clone()));
            block.insert(
                "teams".into(),
                Value::Array(teams.iter().cloned().map(Value::String).collect()),
            );
        }
        // Verbatim: the runner refuses an issuer with surrounding whitespace
        // rather than normalising it, so it is not trimmed here either.
        if let Some(iss) = claims
            .extra
            .get(super::jwt::IDP_ISS_CLAIM)
            .and_then(Value::as_str)
            .filter(|iss| !iss.is_empty())
        {
            block.insert("iss".into(), Value::String(iss.to_string()));
        }
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

/// Upper bound on the team slugs a caller block carries.
pub const MAX_TEAMS: usize = 50;

/// The team slugs a `team_slugs` claim yields: a JSON array whose string entries are
/// DNS labels (1-63 chars of `a-z0-9-`, not starting or ending with `-`).
/// Invalid entries are dropped, the rest sorted, de-duplicated and capped at
/// [`MAX_TEAMS`]. Anything that is not an array yields no teams.
pub fn verified_teams(claim: Option<&Value>) -> Vec<String> {
    let Some(items) = claim.and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut slugs: Vec<String> = items
        .iter()
        .filter_map(Value::as_str)
        .filter(|slug| is_dns_label(slug))
        .map(str::to_string)
        .collect();
    slugs.sort();
    slugs.dedup();
    slugs.truncate(MAX_TEAMS);
    slugs
}

fn is_dns_label(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 63
        && !slug.starts_with('-')
        && !slug.ends_with('-')
        && slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
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
            Some("evil"),
            json!({"groups": ["hr", "ops"], "role": "manager", "email": "a@x", "team_slugs": ["ops"]}),
        ));
        assert_eq!(
            block,
            json!({
                "user_verified": true, "sub": "alice", "team": "ops", "teams": ["ops"],
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
            json!({"groups": ["admins"], "role": "admin", "team_slugs": ["ops"]}),
        ));
        assert_eq!(block, json!({"user_verified": false, "sub": "alice"}));
    }

    #[test]
    fn verified_teams_are_published_as_the_first_sorted_slug_and_the_list() {
        let block = caller_block(&claims(
            true,
            None,
            json!({"team_slugs": ["sales", "ops", "ops"]}),
        ));
        assert_eq!(block["team"], json!("ops"));
        assert_eq!(block["teams"], json!(["ops", "sales"]));
    }

    #[test]
    fn a_verified_token_without_a_teams_claim_carries_no_team() {
        let block = caller_block(&claims(true, None, json!({"groups": ["hr"]})));
        assert!(block.get("team").is_none() && block.get("teams").is_none());
    }

    #[test]
    fn the_request_controlled_context_team_is_never_identity() {
        // `ctx.team` is parsed from `?team=` and must not reach the block,
        // verified or not, with or without a `team_slugs` claim.
        for verified in [true, false] {
            let block = caller_block(&claims(verified, Some("evil"), json!({})));
            assert!(block.get("team").is_none(), "{block}");
            assert!(block.get("teams").is_none(), "{block}");
        }
        let block = caller_block(&claims(true, Some("evil"), json!({"team_slugs": ["ops"]})));
        assert_eq!(block["team"], json!("ops"));
        assert_eq!(block["teams"], json!(["ops"]));
    }

    #[test]
    fn invalid_slugs_are_dropped_and_the_list_is_capped() {
        let teams = verified_teams(Some(&json!([
            "ok",
            "Upper",
            "-lead",
            "trail-",
            "has space",
            "a.b",
            "",
            7,
            null,
            "x".repeat(64)
        ])));
        assert_eq!(teams, vec!["ok".to_string()]);
        assert!(verified_teams(Some(&json!("ops"))).is_empty());
        assert!(verified_teams(None).is_empty());

        let many: Vec<String> = (0..80).map(|i| format!("t{i:02}")).collect();
        let capped = verified_teams(Some(&json!(many)));
        assert_eq!(capped.len(), MAX_TEAMS);
        assert_eq!(capped[0], "t00");
    }

    #[test]
    fn the_issuers_opaque_teams_ids_are_never_used_as_slugs() {
        let block = caller_block(&claims(true, None, json!({"teams": ["team_ab12cd"]})));
        assert!(block.get("team").is_none() && block.get("teams").is_none());
        let block = caller_block(&claims(
            true,
            None,
            json!({"teams": ["team_ab12cd"], "team_slugs": ["ops"]}),
        ));
        assert_eq!(block["teams"], json!(["ops"]));
    }

    #[test]
    fn a_token_with_only_invalid_slugs_carries_no_team() {
        let block = caller_block(&claims(true, None, json!({"team_slugs": ["BAD", "-x"]})));
        assert!(block.get("team").is_none() && block.get("teams").is_none());
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
    fn a_verified_token_carries_the_identity_providers_issuer_verbatim() {
        let block = caller_block(&claims(
            true,
            None,
            json!({"idp_iss": "https://Acme.greentic-id.com/"}),
        ));
        assert_eq!(
            block,
            json!({"user_verified": true, "sub": "alice", "iss": "https://Acme.greentic-id.com/"})
        );
    }

    #[test]
    fn an_unverified_token_never_asserts_an_issuer() {
        let block = caller_block(&claims(
            false,
            None,
            json!({"idp_iss": "https://acme.greentic-id.com"}),
        ));
        assert_eq!(block, json!({"user_verified": false, "sub": "alice"}));
    }

    #[test]
    fn an_empty_or_non_string_issuer_is_omitted() {
        for bad in [json!(""), json!(7), json!(["https://a"]), json!(null)] {
            let block = caller_block(&claims(true, None, json!({ "idp_iss": bad })));
            assert!(block.get("iss").is_none(), "{block}");
        }
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
