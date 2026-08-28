use super::*;

#[test]
fn known_codes_round_trip_through_their_wire_spelling() {
    for code in every_known_code() {
        let encoded = serde_json::to_string(&code).expect("encode");
        let decoded: ErrorCode = serde_json::from_str(&encoded).expect("decode");
        assert_eq!(code, decoded);
    }
}

/// Every code this build spells, with the enum itself as the anchor.
///
/// The match is why this exists: adding a variant stops it compiling, so a new
/// code cannot be minted without being round-tripped. A hand-written list is
/// how `unknown_provider` came to ship without this covering it.
fn every_known_code() -> Vec<ErrorCode> {
    let codes = vec![
        ErrorCode::MethodNotFound,
        ErrorCode::InvalidParams,
        ErrorCode::MalformedHello,
        ErrorCode::ProtocolViolation,
        ErrorCode::Busy,
        ErrorCode::CommandIdConflict,
        ErrorCode::UnknownProvider,
        ErrorCode::SessionNotContinuable,
    ];
    for code in &codes {
        match code {
            ErrorCode::MethodNotFound
            | ErrorCode::InvalidParams
            | ErrorCode::MalformedHello
            | ErrorCode::ProtocolViolation
            | ErrorCode::Busy
            | ErrorCode::CommandIdConflict
            | ErrorCode::UnknownProvider
            | ErrorCode::SessionNotContinuable => {}
            // Not a spelling this build owns: it is whatever a peer sent.
            ErrorCode::Unknown(_) => unreachable!("the list above names no unknown code"),
        }
    }
    codes
}

#[test]
fn an_unknown_code_survives_a_round_trip_unchanged() {
    let decoded: ErrorCode = serde_json::from_str(r#""session_gone""#).expect("decode");

    assert_eq!(decoded, ErrorCode::Unknown("session_gone".to_owned()));
    assert_eq!(
        serde_json::to_string(&decoded).expect("encode"),
        r#""session_gone""#
    );
}

/// A peer that does not know `busy` reads it as an unknown code and keeps
/// working — the same seam every future code arrives through.
#[test]
fn a_peer_that_does_not_know_busy_still_decodes_it() {
    let raw = ErrorCode::Busy.as_str().to_owned();

    assert_eq!(ErrorCode::from(raw.clone()), ErrorCode::Busy);
    assert_eq!(
        String::from(ErrorCode::Unknown(raw.clone())),
        raw,
        "an older peer round-trips it unchanged"
    );
}
