use super::*;

#[test]
fn known_codes_round_trip_through_their_wire_spelling() {
    for code in [
        ErrorCode::MethodNotFound,
        ErrorCode::InvalidParams,
        ErrorCode::MalformedHello,
        ErrorCode::ProtocolViolation,
    ] {
        let encoded = serde_json::to_string(&code).expect("encode");
        let decoded: ErrorCode = serde_json::from_str(&encoded).expect("decode");
        assert_eq!(code, decoded);
    }
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
