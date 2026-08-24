use std::time::Duration;

use super::*;

fn grant() -> AttachGrant {
    AttachGrant {
        session: CorralSessionId::mint(),
        run: RunId::mint(),
    }
}

#[test]
fn a_freshly_issued_token_opens_what_it_was_issued_for() {
    let mut tokens = AttachTokens::new();
    let grant = grant();

    let token = tokens.issue(grant).expect("minted");

    assert_eq!(tokens.redeem(&token).expect("redeemable"), grant);
}

/// Redemption is one step, so two clients cannot both validate the same token
/// before either consumes it.
#[test]
fn a_token_opens_exactly_one_channel() {
    let mut tokens = AttachTokens::new();
    let token = tokens.issue(grant()).expect("minted");

    assert!(tokens.redeem(&token).is_ok());

    assert_eq!(tokens.redeem(&token), Err(AttachRefused));
    assert_eq!(tokens.outstanding(), 0);
}

/// Consumption is final. A caller whose snapshot then fails asks for another
/// token rather than reviving a spent one — there is no branch where a
/// capability comes back.
#[test]
fn a_spent_token_is_not_revived_by_a_failure_after_redemption() {
    let mut tokens = AttachTokens::new();
    let token = tokens.issue(grant()).expect("minted");
    let _grant = tokens.redeem(&token).expect("redeemable");

    // Whatever the caller does next, the token is gone.
    assert_eq!(tokens.redeem(&token), Err(AttachRefused));
}

#[test]
fn a_token_past_its_life_is_refused() {
    let mut tokens = AttachTokens::new();
    let issued_at = std::time::Instant::now();
    let token = tokens.issue_at(grant(), issued_at).expect("minted");

    let refusal = tokens.redeem_at(
        &token,
        issued_at + ATTACH_TOKEN_TTL + Duration::from_millis(1),
    );

    assert_eq!(refusal, Err(AttachRefused));
}

#[test]
fn a_token_within_its_life_is_honoured() {
    let mut tokens = AttachTokens::new();
    let issued_at = std::time::Instant::now();
    let expected = grant();
    let token = tokens.issue_at(expected, issued_at).expect("minted");

    let redeemed = tokens.redeem_at(
        &token,
        issued_at + ATTACH_TOKEN_TTL - Duration::from_millis(1),
    );

    assert_eq!(redeemed, Ok(expected));
}

/// A Session outlives its Runs, so a token minted for one process must never
/// open the terminal of the process that replaced it. Binding to the Session
/// alone would let exactly that happen.
#[test]
fn a_token_names_the_run_it_was_issued_for_not_just_the_session() {
    let mut tokens = AttachTokens::new();
    let session = CorralSessionId::mint();
    let first_run = AttachGrant {
        session,
        run: RunId::mint(),
    };
    let second_run = AttachGrant {
        session,
        run: RunId::mint(),
    };

    let token = tokens.issue(first_run).expect("minted");

    let redeemed = tokens.redeem(&token).expect("redeemable");
    assert_eq!(redeemed, first_run);
    assert_ne!(
        redeemed, second_run,
        "the token resolved to a different Run of the same Session"
    );
}

#[test]
fn two_tokens_are_never_the_same_value() {
    let mut tokens = AttachTokens::new();

    let first = tokens.issue(grant()).expect("minted");
    let second = tokens.issue(grant()).expect("minted");

    assert_ne!(first.to_wire(), second.to_wire());
}

#[test]
fn a_token_survives_its_wire_form() {
    let mut tokens = AttachTokens::new();
    let expected = grant();
    let token = tokens.issue(expected).expect("minted");

    let wire = token.to_wire();
    let parsed = AttachToken::from_wire(&wire).expect("a well-formed token");

    assert_eq!(tokens.redeem(&parsed), Ok(expected));
}

#[test]
fn a_malformed_wire_token_is_not_a_token() {
    assert!(AttachToken::from_wire("").is_none());
    assert!(AttachToken::from_wire("not-hex-not-hex-not-hex-not-hex-").is_none());
    assert!(AttachToken::from_wire("abcd").is_none());
}

/// A capability in a log outlives the thirty seconds it was meant to.
#[test]
fn a_token_never_prints_its_value() {
    let mut tokens = AttachTokens::new();
    let token = tokens.issue(grant()).expect("minted");

    let printed = format!("{token:?}");

    assert!(!printed.contains(&token.to_wire()), "{printed}");
}

#[test]
fn sweeping_reclaims_only_what_can_no_longer_be_redeemed() {
    let mut tokens = AttachTokens::new();
    let now = std::time::Instant::now();
    let stale = tokens
        .issue_at(grant(), now - ATTACH_TOKEN_TTL)
        .expect("minted");
    let live = tokens.issue_at(grant(), now).expect("minted");

    tokens.discard_expired_at(now);

    assert_eq!(tokens.outstanding(), 1);
    assert_eq!(tokens.redeem_at(&stale, now), Err(AttachRefused));
    assert!(tokens.redeem_at(&live, now).is_ok());
}
