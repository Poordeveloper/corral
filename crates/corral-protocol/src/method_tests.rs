use super::*;
use serde_json::json;

#[test]
fn an_empty_session_list_encodes_as_an_empty_array() {
    let encoded = serde_json::to_string(&SessionListResult::default()).expect("encode");

    assert_eq!(encoded, r#"{"sessions":[]}"#);
}

/// The hand-built wire values must stay the encodings of their types, or
/// the daemon would answer with something the contract does not describe.
#[test]
fn the_hand_built_wire_values_match_their_types() {
    assert_eq!(
        PingResult::wire_value(),
        serde_json::to_value(PingResult::default()).expect("encode")
    );
    assert_eq!(
        SessionListResult::empty_wire_value(),
        serde_json::to_value(SessionListResult::default()).expect("encode")
    );
}

#[test]
fn a_future_session_shape_still_decodes() {
    let decoded: SessionListResult =
        serde_json::from_str(r#"{"sessions":[{"id":"s1","attention":"needs_you"}]}"#)
            .expect("decode");

    assert_eq!(decoded.sessions.len(), 1);
}

#[test]
fn baseline_methods_accept_absent_and_null_params_only() {
    assert!(accepts_no_params(None));
    assert!(accepts_no_params(Some(&Value::Null)));
    assert!(!accepts_no_params(Some(&json!({"workspace": "x"}))));
}
