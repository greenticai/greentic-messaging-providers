use super::*;

const SECRET: &str = "test-signing-secret";
const NOW: i64 = 1_800_000_000;

fn sign(secret: &str, ts: &str, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("hmac");
    mac.update(format!("v0:{ts}:").as_bytes());
    mac.update(body);
    format!("v0={}", hex_encode(&mac.finalize().into_bytes()))
}

fn check(ts: i64, body: &[u8]) -> Result<(), VerifyError> {
    let ts_s = ts.to_string();
    verify_request(SECRET, &sign(SECRET, &ts_s, body), &ts_s, body, NOW)
}

#[test]
fn constant_time_eq_compares_every_byte() {
    assert!(constant_time_eq(b"", b""));
    assert!(constant_time_eq(b"v0=abc", b"v0=abc"));
    assert!(!constant_time_eq(b"v0=abc", b"v0=abd"));
    assert!(!constant_time_eq(b"v0=abc", b"v0=ab"));
    assert!(!constant_time_eq(b"a", b""));
    // A difference in the first and in the last byte are both found.
    assert!(!constant_time_eq(b"xbcdef", b"abcdef"));
    assert!(!constant_time_eq(b"abcdex", b"abcdef"));
}

#[test]
fn a_correct_fresh_signature_is_accepted() {
    assert_eq!(check(NOW, b"{\"a\":1}"), Ok(()));
    assert_eq!(check(NOW - MAX_SKEW_SECS, b"x"), Ok(()));
    assert_eq!(check(NOW + MAX_SKEW_SECS, b"x"), Ok(()));
}

#[test]
fn a_stale_timestamp_is_refused_even_with_a_valid_signature() {
    assert_eq!(
        check(NOW - MAX_SKEW_SECS - 1, b"x"),
        Err(VerifyError::StaleTimestamp)
    );
}

#[test]
fn a_future_timestamp_is_refused_even_with_a_valid_signature() {
    assert_eq!(
        check(NOW + MAX_SKEW_SECS + 1, b"x"),
        Err(VerifyError::FutureTimestamp)
    );
}

#[test]
fn a_wrong_or_malformed_signature_is_refused() {
    let ts = NOW.to_string();
    let good = sign(SECRET, &ts, b"x");
    for bad in [
        sign("other-secret", &ts, b"x"),
        format!("{good}0"),
        good[..good.len() - 1].to_string(),
        String::new(),
        "v0=".to_string(),
    ] {
        assert_eq!(
            verify_request(SECRET, &bad, &ts, b"x", NOW),
            Err(VerifyError::InvalidSignature),
            "{bad}"
        );
    }
    // The body is covered.
    assert_eq!(
        verify_request(SECRET, &good, &ts, b"y", NOW),
        Err(VerifyError::InvalidSignature)
    );
}

#[test]
fn the_timestamp_is_covered_by_the_signature() {
    let signed_for = (NOW - 10).to_string();
    let sig = sign(SECRET, &signed_for, b"x");
    assert_eq!(
        verify_request(SECRET, &sig, &NOW.to_string(), b"x", NOW),
        Err(VerifyError::InvalidSignature)
    );
}

#[test]
fn an_empty_secret_or_garbage_timestamp_is_refused() {
    let ts = NOW.to_string();
    assert_eq!(
        verify_request("", &sign("", &ts, b"x"), &ts, b"x", NOW),
        Err(VerifyError::InvalidSecret)
    );
    assert_eq!(
        verify_request(SECRET, "v0=00", "soon", b"x", NOW),
        Err(VerifyError::InvalidTimestamp)
    );
    assert_eq!(
        verify_request(SECRET, "v0=00", "", b"x", NOW),
        Err(VerifyError::InvalidTimestamp)
    );
}

#[test]
fn non_utf8_bodies_verify_over_raw_bytes() {
    assert_eq!(check(NOW, &[0xff, 0xfe, 0x00, 0x41]), Ok(()));
}

// --- caller derivation -------------------------------------------------

fn dm_event() -> Value {
    json!({
        "type": "event_callback",
        "team_id": "T111",
        "event": {
            "type": "message", "channel": "D999", "channel_type": "im",
            "user": "U123ABC", "text": "hi"
        }
    })
}

#[test]
fn a_dm_message_yields_the_exact_caller_block() {
    assert_eq!(
        caller_for_event(&dm_event()),
        Some(json!({"user_verified": true, "sub": "U123ABC", "iss": "slack:T111"}))
    );
}

#[test]
fn channel_types_other_than_im_get_no_caller() {
    for (channel, ty) in [
        ("C999", "channel"),
        ("G999", "group"),
        ("G998", "mpim"),
        // A D-prefixed id with a non-im type is still refused.
        ("D999", "channel"),
    ] {
        let mut body = dm_event();
        body["event"]["channel"] = json!(channel);
        body["event"]["channel_type"] = json!(ty);
        assert_eq!(caller_for_event(&body), None, "{channel}/{ty}");
    }
    // No channel at all.
    let mut body = dm_event();
    body["event"].as_object_mut().unwrap().remove("channel");
    assert_eq!(caller_for_event(&body), None);
}

#[test]
fn without_a_channel_type_only_a_d_channel_counts() {
    let mut body = dm_event();
    body["event"]
        .as_object_mut()
        .unwrap()
        .remove("channel_type");
    assert!(caller_for_event(&body).is_some());
    body["event"]["channel"] = json!("C999");
    assert_eq!(caller_for_event(&body), None);
    body["event"]["channel"] = json!("G999");
    assert_eq!(caller_for_event(&body), None);
}

#[test]
fn bot_and_app_events_get_no_caller() {
    for patch in [
        json!({"bot_id": "B1"}),
        json!({"bot_profile": {"id": "B1"}}),
        json!({"subtype": "bot_message"}),
        json!({"subtype": "message_changed"}),
        json!({"subtype": "message_deleted"}),
    ] {
        let mut body = dm_event();
        for (k, v) in patch.as_object().unwrap() {
            body["event"][k] = v.clone();
        }
        assert_eq!(caller_for_event(&body), None, "{patch}");
    }
}

#[test]
fn events_without_a_user_or_of_another_type_get_no_caller() {
    let mut body = dm_event();
    body["event"].as_object_mut().unwrap().remove("user");
    assert_eq!(caller_for_event(&body), None);

    let mut body = dm_event();
    body["event"]["type"] = json!("app_home_opened");
    assert_eq!(caller_for_event(&body), None);

    // A flat payload outside an event_callback is never trusted.
    let flat = json!({"channel": "D999", "channel_type": "im", "user": "U123ABC"});
    assert_eq!(caller_for_event(&flat), None);
}

#[test]
fn a_file_share_in_a_dm_is_still_a_user_turn() {
    let mut body = dm_event();
    body["event"]["subtype"] = json!("file_share");
    assert!(caller_for_event(&body).is_some());
}

#[test]
fn issuer_selection_for_grid_connect_and_plain_workspaces() {
    // Plain workspace.
    assert_eq!(
        caller_block("U1A", Some("T111"), None, Some("T111")).unwrap()["iss"],
        "slack:T111"
    );
    // Enterprise Grid: ids are unique across the org.
    assert_eq!(
        caller_block("U1A", Some("T111"), Some("E555"), Some("T111")).unwrap()["iss"],
        "slack:E555"
    );
    assert_eq!(
        caller_block("U1A", Some("T111"), Some("E555"), None).unwrap()["iss"],
        "slack:E555"
    );
    // Slack Connect: the external user's own team, not ours.
    assert_eq!(
        caller_block("U1A", Some("T111"), None, Some("T222")).unwrap()["iss"],
        "slack:T222"
    );
    assert_eq!(
        caller_block("U1A", Some("T111"), Some("E555"), Some("T222")).unwrap()["iss"],
        "slack:T222"
    );
    // Only a user team.
    assert_eq!(
        caller_block("U1A", None, None, Some("T222")).unwrap()["iss"],
        "slack:T222"
    );
}

#[test]
fn grid_enterprise_id_is_read_from_the_callback() {
    let mut body = dm_event();
    body["enterprise_id"] = json!("E555");
    assert_eq!(caller_for_event(&body).unwrap()["iss"], "slack:E555");

    let mut body = dm_event();
    body.as_object_mut().unwrap().remove("team_id");
    body["authorizations"] = json!([{"team_id": "T777", "enterprise_id": "E888"}]);
    assert_eq!(caller_for_event(&body).unwrap()["iss"], "slack:E888");
}

#[test]
fn connect_user_team_is_read_from_the_event() {
    let mut body = dm_event();
    body["event"]["user_team"] = json!("T222");
    assert_eq!(caller_for_event(&body).unwrap()["iss"], "slack:T222");
}

#[test]
fn malformed_ids_omit_the_block() {
    for user in [
        "", "u123", "X123", "U", "U 12", "U12\n3", " U123", "U123 ", "U-12",
    ] {
        assert_eq!(
            caller_block(user, Some("T1AB"), None, None),
            None,
            "{user:?}"
        );
    }
    for team in ["", "t1", "A1", "T", "T 1", "T1\u{0}"] {
        assert_eq!(
            caller_block("U1AB", Some(team), None, None),
            None,
            "{team:?}"
        );
    }
    // No issuer at all.
    assert_eq!(caller_block("U1AB", None, None, None), None);
    // Over-long.
    let long = format!("U{}", "A".repeat(80));
    assert_eq!(caller_block(&long, Some("T1AB"), None, None), None);
    // A malformed enterprise falls back to the team rather than being sent.
    assert_eq!(
        caller_block("U1AB", Some("T1AB"), Some("bad id"), None).unwrap()["iss"],
        "slack:T1AB"
    );
}

#[test]
fn block_actions_in_a_dm_get_a_caller_and_in_a_channel_do_not() {
    let click = json!({
        "type": "block_actions",
        "user": {"id": "U123ABC", "team_id": "T111"},
        "team": {"id": "T111"},
        "channel": {"id": "D999"},
        "actions": []
    });
    assert_eq!(
        caller_for_block_actions(&click),
        Some(json!({"user_verified": true, "sub": "U123ABC", "iss": "slack:T111"}))
    );
    let mut in_channel = click.clone();
    in_channel["channel"]["id"] = json!("C999");
    assert_eq!(caller_for_block_actions(&in_channel), None);
    let mut no_channel = click.clone();
    no_channel.as_object_mut().unwrap().remove("channel");
    assert_eq!(caller_for_block_actions(&no_channel), None);

    let mut grid = click.clone();
    grid["enterprise"] = json!({"id": "E555"});
    assert_eq!(
        caller_for_block_actions(&grid).unwrap()["iss"],
        "slack:E555"
    );
}

#[test]
fn view_submission_uses_the_origin_channel() {
    let sub = json!({
        "type": "view_submission",
        "user": {"id": "U123ABC", "team_id": "T111"},
        "team": {"id": "T111"}
    });
    assert_eq!(
        caller_for_view_submission(&sub, Some("D999")),
        Some(json!({"user_verified": true, "sub": "U123ABC", "iss": "slack:T111"}))
    );
    assert_eq!(caller_for_view_submission(&sub, Some("C999")), None);
    assert_eq!(caller_for_view_submission(&sub, None), None);
    assert_eq!(caller_for_view_submission(&sub, Some("")), None);
}
