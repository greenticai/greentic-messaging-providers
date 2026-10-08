use super::*;
use crate::directline::jwt::{DirectLineContext, TokenClaims};
use crate::directline::state::ConversationState;
use base64::{Engine as _, engine::general_purpose};
use serde_json::{Map, Value};

const CONV: &str = "conv-a";

fn ctx() -> DirectLineContext {
    DirectLineContext {
        env: "e".into(),
        tenant: "t".into(),
        team: None,
    }
}

fn other_ctx() -> DirectLineContext {
    DirectLineContext {
        env: "e".into(),
        tenant: "other".into(),
        team: None,
    }
}

fn claims(sub: &str, verified: bool, conv: Option<&str>, ctx: DirectLineContext) -> TokenClaims {
    TokenClaims {
        iss: "greentic.webchat".into(),
        aud: "directline".into(),
        sub: sub.into(),
        iat: 0,
        nbf: 0,
        exp: i64::MAX,
        ctx,
        conv: conv.map(str::to_string),
        verified,
        extra: Map::new(),
    }
}

fn owned(sub: &str, verified: bool) -> Lookup {
    Lookup::Found(ConversationState::new_owned(ctx(), sub, verified))
}

fn legacy() -> Lookup {
    Lookup::Found(ConversationState::new(ctx()))
}

fn refusal(result: Result<Authorized, Refusal>) -> Refusal {
    match result {
        Ok(authorized) => panic!("expected a refusal, got {authorized:?}"),
        Err(refusal) => refusal,
    }
}

fn body(out: &greentic_types::messaging::universal_dto::HttpOutV1) -> Value {
    let bytes = general_purpose::STANDARD
        .decode(&out.body_b64)
        .expect("base64 body");
    serde_json::from_slice(&bytes).expect("json body")
}

#[test]
fn bound_token_on_its_conversation_is_allowed() {
    for lookup in [
        owned("guest-1", false),
        owned("acme:users:7", true),
        legacy(),
    ] {
        let token = claims("anyone", false, Some(CONV), ctx());
        assert!(matches!(
            authorize(&token, CONV, lookup),
            Ok(Authorized::Bound(_))
        ));
    }
}

#[test]
fn bound_token_on_another_conversation_is_wrong_conversation() {
    let token = claims("guest-1", false, Some("conv-b"), ctx());
    assert_eq!(
        refusal(authorize(&token, CONV, owned("guest-1", false))),
        Refusal::WrongConversation
    );
}

#[test]
fn bound_token_on_a_missing_conversation_is_not_found() {
    let token = claims("guest-1", false, Some(CONV), ctx());
    assert_eq!(
        refusal(authorize(&token, CONV, Lookup::Missing)),
        Refusal::NotFound
    );
}

#[test]
fn bound_token_with_another_ctx_is_context_mismatch() {
    let token = claims("guest-1", false, Some(CONV), other_ctx());
    assert_eq!(
        refusal(authorize(&token, CONV, owned("guest-1", false))),
        Refusal::TokenContextMismatch
    );
}

#[test]
fn verified_unbound_token_matching_a_verified_owner_is_allowed() {
    let token = claims("acme:users:7", true, None, ctx());
    assert!(matches!(
        authorize(&token, CONV, owned("acme:users:7", true)),
        Ok(Authorized::VerifiedOwner(_))
    ));
}

#[test]
fn verified_unbound_token_with_another_sub_is_owner_required() {
    let token = claims("acme:users:8", true, None, ctx());
    assert_eq!(
        refusal(authorize(&token, CONV, owned("acme:users:7", true))),
        Refusal::OwnerRequired
    );
}

#[test]
fn verified_owner_match_is_exact_byte_equality() {
    for sub in ["ACME:users:7", " acme:users:7", "acme:users:7 "] {
        let token = claims(sub, true, None, ctx());
        assert_eq!(
            refusal(authorize(&token, CONV, owned("acme:users:7", true))),
            Refusal::OwnerRequired,
            "{sub:?}"
        );
    }
}

#[test]
fn unverified_unbound_token_with_the_owners_sub_is_owner_required() {
    let token = claims("guest-1", false, None, ctx());
    assert_eq!(
        refusal(authorize(&token, CONV, owned("guest-1", false))),
        Refusal::OwnerRequired
    );
    let token = claims("acme:users:7", false, None, ctx());
    assert_eq!(
        refusal(authorize(&token, CONV, owned("acme:users:7", true))),
        Refusal::OwnerRequired
    );
}

#[test]
fn verified_unbound_token_on_an_anonymous_conversation_with_equal_sub_is_owner_required() {
    let token = claims("guest-1", true, None, ctx());
    assert_eq!(
        refusal(authorize(&token, CONV, owned("guest-1", false))),
        Refusal::OwnerRequired
    );
}

#[test]
fn unbound_token_on_a_legacy_conversation_is_owner_required() {
    for verified in [true, false] {
        let token = claims("acme:users:7", verified, None, ctx());
        assert_eq!(
            refusal(authorize(&token, CONV, legacy())),
            Refusal::OwnerRequired
        );
    }
}

#[test]
fn unbound_token_on_a_missing_conversation_answers_exactly_like_a_refused_one() {
    let token = claims("acme:users:8", true, None, ctx());
    let missing = refusal(authorize(&token, CONV, Lookup::Missing)).into_response();
    let refused = refusal(authorize(&token, CONV, owned("acme:users:7", true))).into_response();
    assert_eq!(missing, refused);
}

#[test]
fn verified_unbound_token_with_another_ctx_is_owner_required() {
    let token = claims("acme:users:7", true, None, other_ctx());
    assert_eq!(
        refusal(authorize(&token, CONV, owned("acme:users:7", true))),
        Refusal::OwnerRequired
    );
}

#[test]
fn owner_required_answers_the_documented_body() {
    let out = Refusal::OwnerRequired.into_response();
    assert_eq!(out.status, 403);
    assert_eq!(
        body(&out),
        serde_json::json!({
            "error": "forbidden",
            "code": "ConversationOwnerRequired",
            "message": "this conversation belongs to another session; start a new conversation",
        })
    );
}

#[test]
fn the_existing_refusals_keep_their_status_and_message_and_gain_a_code() {
    let wrong = Refusal::WrongConversation.into_response();
    assert_eq!(wrong.status, 403);
    assert_eq!(body(&wrong)["code"], "WrongConversation");
    assert_eq!(
        body(&wrong)["message"],
        "token bound to different conversation"
    );

    let ctx_mismatch = Refusal::TokenContextMismatch.into_response();
    assert_eq!(ctx_mismatch.status, 403);
    assert_eq!(body(&ctx_mismatch)["code"], "TokenContextMismatch");
    assert_eq!(body(&ctx_mismatch)["message"], "token context mismatch");

    let missing = Refusal::NotFound.into_response();
    assert_eq!(missing.status, 404);
    assert_eq!(body(&missing)["message"], "conversation not found");
}
