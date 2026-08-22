use super::*;

#[test]
fn minting_is_unique() {
    assert_ne!(CorralSessionId::mint(), CorralSessionId::mint());
}

#[test]
fn an_identity_round_trips_through_its_text_form() {
    let id = RunId::mint();
    let parsed: RunId = id.to_string().parse().expect("round trip");
    assert_eq!(id, parsed);
}

/// Identity is opaque: two identities minted in a known order carry nothing
/// that recovers that order. A time-ordered UUID would, which is why D1
/// rejects one.
#[test]
fn identity_encodes_no_minting_order() {
    let first = CorralSessionId::mint().as_uuid();
    let second = CorralSessionId::mint().as_uuid();

    assert_eq!(
        first.get_version_num(),
        4,
        "a random UUID, not a time-ordered one"
    );
    assert_eq!(second.get_version_num(), 4);
}

#[test]
fn text_corral_did_not_mint_is_not_an_identity() {
    let error = "session-7".parse::<CorralSessionId>().expect_err("refused");

    assert_eq!(error.raw, "session-7");
    assert!(error.to_string().contains("a Corral session id"));
}

/// The identities are distinct types, so one can never be passed where
/// another is expected — the reason they are not one `Uuid` alias.
#[test]
fn the_identity_types_do_not_interchange() {
    let session = CorralSessionId::mint();
    let run = RunId::from_uuid(session.as_uuid());

    assert_eq!(session.as_uuid(), run.as_uuid());
    assert_eq!(session.to_string(), run.to_string());
}
