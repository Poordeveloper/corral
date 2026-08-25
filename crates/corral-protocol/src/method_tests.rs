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

/// `command_id` is required. A request without it is not an old peer being
/// tolerated: without one, a lost response makes a client retry and the retry
/// starts a second agent nobody asked for.
#[test]
fn session_new_without_a_command_id_does_not_decode() {
    let without = json!({ "argv": ["/bin/sh"], "rows": 24, "cols": 80 });

    assert!(serde_json::from_value::<SessionNewParams>(without).is_err());
}

/// Everything else about the shape is additive-tolerant, so a newer client
/// sending a field this build does not know stays decodable.
#[test]
fn session_new_survives_a_field_this_build_does_not_know() {
    let newer = json!({
        "command_id": "cmd-1",
        "argv": ["/bin/sh", "-c", "sleep 30"],
        "cwd": "/work",
        "rows": 24,
        "cols": 80,
        "environment": {"TERM": "xterm"},
    });

    let decoded: SessionNewParams = serde_json::from_value(newer).expect("decode");

    assert_eq!(decoded.command_id, "cmd-1");
    assert_eq!(decoded.argv, ["/bin/sh", "-c", "sleep 30"]);
    assert_eq!(decoded.cwd.as_deref(), Some("/work"));
}

/// The optional fields are absent rather than defaulted to a size Corral would
/// have to invent: absence means the caller has no preference, and a zero is
/// not a geometry.
#[test]
fn session_new_carries_absence_rather_than_a_substituted_size() {
    let minimal = json!({ "command_id": "cmd-1", "argv": ["/bin/sh"] });

    let decoded: SessionNewParams = serde_json::from_value(minimal).expect("decode");

    assert_eq!(decoded.cwd, None);
    assert_eq!(decoded.rows, None);
    assert_eq!(decoded.cols, None);
}

/// The wire spelling is the type's own, in both directions. A daemon that
/// encoded one word and a client that read another would agree in the
/// handshake and disagree about whether a session can be opened.
#[test]
fn terminal_access_round_trips_through_its_wire_spelling() {
    for access in [TerminalAccess::Available, TerminalAccess::Unavailable] {
        assert_eq!(TerminalAccess::from_wire(access.as_str()), Some(access));
        assert_eq!(
            serde_json::to_value(access).expect("encode"),
            json!(access.as_str())
        );
    }
}

/// The absence rule, at the one place it is decided: a peer that does not send
/// the field has said nothing, and nothing is not "unavailable"
/// (AGENTS.md §Protocol).
#[test]
fn a_session_without_terminal_access_decodes_as_unknown() {
    let older = json!({
        "session_id": "s1",
        "title": "sh",
        "execution_state": "running",
    });

    let decoded: SessionListItem = serde_json::from_value(older).expect("decode");

    assert_eq!(decoded.terminal_access, None);
}

/// A value from a version that knows more than this one. It decodes, and it
/// decodes as unknown — refusing the item would lose the session from the
/// list, and reading it as a refusal would disable Open on a word this build
/// never understood.
#[test]
fn an_unrecognised_terminal_access_decodes_as_unknown() {
    for carried in [
        json!("degraded"),
        json!(null),
        json!(7),
        json!({"state": "unavailable"}),
    ] {
        let item = json!({
            "session_id": "s1",
            "title": "sh",
            "execution_state": "running",
            "terminal_access": carried,
        });

        let decoded: SessionListItem =
            serde_json::from_value(item).unwrap_or_else(|error| panic!("{carried}: {error}"));

        assert_eq!(decoded.terminal_access, None, "{carried}");
    }
}

/// Unknown is not encoded. A daemon that cannot say leaves the field out,
/// because a peer reading a spelled-out "unknown" would have to know that
/// word too.
#[test]
fn an_unknown_terminal_access_is_left_off_the_wire() {
    let item = SessionListItem {
        session_id: "s1".to_owned(),
        title: "sh".to_owned(),
        execution_state: "running".to_owned(),
        terminal_access: None,
    };

    let encoded = serde_json::to_value(&item).expect("encode");

    assert!(encoded.get("terminal_access").is_none(), "{encoded}");
}
