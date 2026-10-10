use super::*;
use crate::directline::jwt::TokenClaims;
use crate::directline::owner::{OWNER_REQUIRED_CODE, OWNER_REQUIRED_MESSAGE};

const KEY: &[u8] = b"test-signing-key";

fn setup() -> (InMemoryStateStore, TestSecretStore) {
    let mut secrets = TestSecretStore::new();
    secrets.insert(TOKEN_SECRET_KEY, KEY);
    (InMemoryStateStore::new(), secrets)
}

fn default_ctx() -> DirectLineContext {
    DirectLineContext {
        env: "default".into(),
        tenant: "default".into(),
        team: None,
    }
}

fn mint_anon(state: &mut InMemoryStateStore, secrets: &TestSecretStore, guest: &str) -> String {
    let request = build_request(
        "POST",
        "/v3/directline/tokens/generate",
        Some("env=default&tenant=default"),
        Some(&json!({"user": {"id": guest}})),
        vec![],
    )
    .expect("request");
    let response = handle_directline_request(&request, state, secrets);
    assert_eq!(response.status, 200);
    decode_body(&response).expect("body")["token"]
        .as_str()
        .expect("token")
        .to_string()
}

fn verified_unbound(sub: &str) -> String {
    issue_token(KEY, default_ctx(), sub, None, true)
        .expect("token")
        .0
}

fn create(
    state: &mut InMemoryStateStore,
    secrets: &TestSecretStore,
    token: &str,
) -> (String, String) {
    let request = build_request(
        "POST",
        "/v3/directline/conversations",
        None,
        None,
        bearer(token),
    )
    .expect("request");
    let response = handle_directline_request(&request, state, secrets);
    assert_eq!(response.status, 201);
    let body = decode_body(&response).expect("body");
    (
        body["conversationId"].as_str().expect("id").to_string(),
        body["token"].as_str().expect("token").to_string(),
    )
}

fn reconnect(
    state: &mut InMemoryStateStore,
    secrets: &TestSecretStore,
    conv_id: &str,
    token: &str,
) -> HttpOutV1 {
    let request = build_request(
        "GET",
        &format!("/v3/directline/conversations/{conv_id}"),
        None,
        None,
        bearer(token),
    )
    .expect("request");
    handle_directline_request(&request, state, secrets)
}

fn post(
    state: &mut InMemoryStateStore,
    secrets: &TestSecretStore,
    conv_id: &str,
    token: &str,
) -> HttpOutV1 {
    let request = build_request(
        "POST",
        &format!("/v3/directline/conversations/{conv_id}/activities"),
        None,
        Some(&json!({"type": "message", "text": "intruder", "from": {"id": "b"}})),
        bearer(token),
    )
    .expect("request");
    handle_directline_request(&request, state, secrets)
}

fn poll(
    state: &mut InMemoryStateStore,
    secrets: &TestSecretStore,
    conv_id: &str,
    token: &str,
) -> HttpOutV1 {
    let request = build_request(
        "GET",
        &format!("/v3/directline/conversations/{conv_id}/activities"),
        None,
        None,
        bearer(token),
    )
    .expect("request");
    handle_directline_request(&request, state, secrets)
}

fn assert_owner_required(response: &HttpOutV1) {
    assert_eq!(response.status, 403);
    let body = decode_body(response).expect("body");
    assert_eq!(
        body,
        json!({"error": "forbidden", "code": OWNER_REQUIRED_CODE, "message": OWNER_REQUIRED_MESSAGE})
    );
    assert!(body.get("token").is_none());
}

fn returned_claims(response: &HttpOutV1) -> TokenClaims {
    let body = decode_body(response).expect("body");
    verify_token(KEY, body["token"].as_str().expect("token")).expect("verifies")
}

#[test]
fn threat_b_cannot_reconnect_to_as_anonymous_conversation_with_its_own_unbound_token() {
    let (mut state, secrets) = setup();
    let a = mint_anon(&mut state, &secrets, "guest-a");
    let (conv_a, _) = create(&mut state, &secrets, &a);
    let b = mint_anon(&mut state, &secrets, "guest-b");
    assert_owner_required(&reconnect(&mut state, &secrets, &conv_a, &b));
}

#[test]
fn threat_b_with_as_guest_id_still_cannot_reconnect() {
    let (mut state, secrets) = setup();
    let a = mint_anon(&mut state, &secrets, "guest-a");
    let (conv_a, _) = create(&mut state, &secrets, &a);
    let b = mint_anon(&mut state, &secrets, "guest-a");
    assert_owner_required(&reconnect(&mut state, &secrets, &conv_a, &b));
    assert_owner_required(&reconnect(&mut state, &secrets, &conv_a, &a));
}

#[test]
fn threat_b_cannot_post_or_poll_with_an_unbound_token() {
    let (mut state, secrets) = setup();
    let a = mint_anon(&mut state, &secrets, "guest-a");
    let (conv_a, a_bound) = create(&mut state, &secrets, &a);
    let b = mint_anon(&mut state, &secrets, "guest-b");
    let before = decode_body(&poll(&mut state, &secrets, &conv_a, &a_bound)).expect("body");

    assert_owner_required(&post(&mut state, &secrets, &conv_a, &b));
    assert_owner_required(&poll(&mut state, &secrets, &conv_a, &b));

    let after = decode_body(&poll(&mut state, &secrets, &conv_a, &a_bound)).expect("body");
    assert_eq!(before, after);
}

#[test]
fn threat_random_conversation_id_is_indistinguishable() {
    let (mut state, secrets) = setup();
    let a = mint_anon(&mut state, &secrets, "guest-a");
    let (conv_a, _) = create(&mut state, &secrets, &a);
    let b = mint_anon(&mut state, &secrets, "guest-b");
    let random = Uuid::new_v4().to_string();
    let real = reconnect(&mut state, &secrets, &conv_a, &b);
    let unknown = reconnect(&mut state, &secrets, &random, &b);
    assert_eq!(real, unknown);
    assert_eq!(
        post(&mut state, &secrets, &conv_a, &b),
        post(&mut state, &secrets, &random, &b)
    );
    assert_eq!(
        poll(&mut state, &secrets, &conv_a, &b),
        poll(&mut state, &secrets, &random, &b)
    );
}

#[test]
fn verified_b_on_a_random_id_matches_verified_b_on_a_real_one() {
    let (mut state, secrets) = setup();
    let (conv_a, _) = create(&mut state, &secrets, &verified_unbound("acme:users:7"));
    let b = verified_unbound("acme:users:8");
    assert_eq!(
        reconnect(&mut state, &secrets, &conv_a, &b),
        reconnect(&mut state, &secrets, &Uuid::new_v4().to_string(), &b)
    );
}

#[test]
fn anonymous_owner_resumes_with_the_bound_token() {
    let (mut state, secrets) = setup();
    let a = mint_anon(&mut state, &secrets, "guest-a");
    let (conv_a, a_bound) = create(&mut state, &secrets, &a);
    let response = reconnect(&mut state, &secrets, &conv_a, &a_bound);
    assert_eq!(response.status, 200);
    assert_eq!(
        returned_claims(&response).conv.as_deref(),
        Some(conv_a.as_str())
    );
    assert_eq!(post(&mut state, &secrets, &conv_a, &a_bound).status, 201);
}

#[test]
fn verified_owner_resumes_with_a_fresh_unbound_verified_token() {
    let (mut state, secrets) = setup();
    let owner = mint_verified_user_token(&mut state, &secrets);
    let (conv_a, _) = create(&mut state, &secrets, &owner);
    let fresh = verified_unbound("acme:users:7");
    let response = reconnect(&mut state, &secrets, &conv_a, &fresh);
    assert_eq!(response.status, 200);
    let claims = returned_claims(&response);
    assert_eq!(claims.conv.as_deref(), Some(conv_a.as_str()));
    assert!(claims.verified);
    assert_eq!(post(&mut state, &secrets, &conv_a, &fresh).status, 201);
    assert_eq!(poll(&mut state, &secrets, &conv_a, &fresh).status, 200);
}

#[test]
fn verified_other_user_is_refused() {
    let (mut state, secrets) = setup();
    let owner = mint_verified_user_token(&mut state, &secrets);
    let (conv_a, _) = create(&mut state, &secrets, &owner);
    let other = verified_unbound("acme:users:8");
    assert_owner_required(&reconnect(&mut state, &secrets, &conv_a, &other));
    assert_owner_required(&post(&mut state, &secrets, &conv_a, &other));
    assert_owner_required(&poll(&mut state, &secrets, &conv_a, &other));
}

#[test]
fn legacy_conversation_only_accepts_a_bound_token() {
    let (mut state, secrets) = setup();
    let conv_id = Uuid::new_v4().to_string();
    let legacy = br#"{"ctx":{"env":"default","tenant":"default","team":null},"next_watermark":0,"layout":2}"#;
    state
        .write(&default_conv_key(&conv_id), legacy)
        .expect("write");
    let bound = issue_token(
        KEY,
        default_ctx(),
        "acme:users:7",
        Some(conv_id.clone()),
        true,
    )
    .expect("token")
    .0;
    assert_eq!(
        reconnect(&mut state, &secrets, &conv_id, &bound).status,
        200
    );
    assert_owner_required(&reconnect(
        &mut state,
        &secrets,
        &conv_id,
        &verified_unbound("acme:users:7"),
    ));
}

#[test]
fn a_new_conversation_records_its_owner() {
    let (mut state, secrets) = setup();
    let a = mint_anon(&mut state, &secrets, "guest-a");
    let (conv_a, _) = create(&mut state, &secrets, &a);
    let stored = read_conversation(&mut state, &default_conv_key(&conv_a)).expect("stored");
    assert_eq!(stored.owner_sub.as_deref(), Some("guest-a"));
    assert!(!stored.owner_verified);

    let (conv_v, _) = create(&mut state, &secrets, &verified_unbound("acme:users:7"));
    let stored = read_conversation(&mut state, &default_conv_key(&conv_v)).expect("stored");
    assert_eq!(stored.owner_sub.as_deref(), Some("acme:users:7"));
    assert!(stored.owner_verified);
}

#[test]
fn refresh_of_a_bound_token_keeps_its_conversation() {
    let (mut state, secrets) = setup();
    let a = mint_anon(&mut state, &secrets, "guest-a");
    let (conv_a, a_bound) = create(&mut state, &secrets, &a);
    let request = build_request(
        "POST",
        "/v3/directline/tokens/refresh",
        None,
        None,
        bearer(&a_bound),
    )
    .expect("request");
    let response = handle_directline_request(&request, &mut state, &secrets);
    assert_eq!(response.status, 200);
    assert_eq!(
        returned_claims(&response).conv.as_deref(),
        Some(conv_a.as_str())
    );
}

#[test]
fn a_corrupt_header_answers_an_unbound_token_exactly_like_a_missing_conversation() {
    let (mut state, secrets) = setup();
    let corrupt_id = Uuid::new_v4().to_string();
    state
        .write(&default_conv_key(&corrupt_id), b"{not json")
        .expect("write");
    let missing_id = Uuid::new_v4().to_string();
    let anon = mint_anon(&mut state, &secrets, "guest-b");
    let verified = verified_unbound("acme:users:8");
    for token in [&anon, &verified] {
        let corrupt = reconnect(&mut state, &secrets, &corrupt_id, token);
        assert_owner_required(&corrupt);
        assert_eq!(corrupt, reconnect(&mut state, &secrets, &missing_id, token));
        assert_eq!(
            post(&mut state, &secrets, &corrupt_id, token),
            post(&mut state, &secrets, &missing_id, token)
        );
        assert_eq!(
            poll(&mut state, &secrets, &corrupt_id, token),
            poll(&mut state, &secrets, &missing_id, token)
        );
    }
}

#[test]
fn a_corrupt_header_is_a_server_error_for_the_token_bound_to_it() {
    let (mut state, secrets) = setup();
    let conv_id = Uuid::new_v4().to_string();
    state
        .write(&default_conv_key(&conv_id), b"{not json")
        .expect("write");
    let bound = issue_token(KEY, default_ctx(), "guest-a", Some(conv_id.clone()), false)
        .expect("token")
        .0;
    assert_eq!(
        reconnect(&mut state, &secrets, &conv_id, &bound).status,
        500
    );
}

fn refresh(state: &mut InMemoryStateStore, secrets: &TestSecretStore, token: &str) -> HttpOutV1 {
    let request = build_request(
        "POST",
        "/v3/directline/tokens/refresh",
        None,
        None,
        bearer(token),
    )
    .expect("request");
    handle_directline_request(&request, state, secrets)
}

#[test]
fn refresh_of_a_token_bound_to_a_missing_conversation_is_refused() {
    let (mut state, secrets) = setup();
    let gone = Uuid::new_v4().to_string();
    let bound = issue_token(KEY, default_ctx(), "guest-a", Some(gone), false)
        .expect("token")
        .0;
    let response = refresh(&mut state, &secrets, &bound);
    assert_eq!(response.status, 404);
    assert!(decode_body(&response).expect("body").get("token").is_none());
}

#[test]
fn refresh_of_a_bound_token_from_another_context_is_refused() {
    let (mut state, secrets) = setup();
    let a = mint_anon(&mut state, &secrets, "guest-a");
    let (conv_a, _) = create(&mut state, &secrets, &a);
    let other_ctx = DirectLineContext {
        env: "default".into(),
        tenant: "other".into(),
        team: None,
    };
    let foreign = issue_token(KEY, other_ctx, "guest-a", Some(conv_a), false)
        .expect("token")
        .0;
    let response = refresh(&mut state, &secrets, &foreign);
    assert_ne!(response.status, 200);
    assert!(decode_body(&response).expect("body").get("token").is_none());
}
