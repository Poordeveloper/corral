use super::*;

fn listed(execution_state: &str, terminal_access: Option<TerminalAccess>) -> SessionListItem {
    SessionListItem {
        session_id: "0f9b6c1a-0000-0000-0000-000000000000".to_owned(),
        title: "sh".to_owned(),
        execution_state: execution_state.to_owned(),
        terminal_access,
    }
}

/// The projection, in full. Table-driven because the regression this exists
/// to prevent is a single arm quietly promoting a runtime fact into a
/// semantic claim.
#[test]
fn execution_state_projects_onto_the_states_corral_may_claim() {
    let cases = [
        ("running", MainState::Unknown, "Running · Status unknown"),
        ("exited", MainState::Exited, "Exited"),
        (
            "unknown",
            MainState::Unknown,
            "Runtime unverified · Status unknown",
        ),
    ];

    for (execution_state, state, line) in cases {
        let presented = present(&listed(execution_state, None));

        assert_eq!(presented.state, state, "{execution_state}");
        assert_eq!(presented.state_line(), line, "{execution_state}");
    }
}

/// The invariant the module exists for. No execution state — including ones
/// this build has never seen — may produce a state that needs semantic
/// evidence nothing has yet.
#[test]
fn no_execution_state_manufactures_a_semantic_status() {
    for execution_state in [
        "running",
        "exited",
        "unknown",
        "working",
        "needs_you",
        "ready",
        "",
    ] {
        let line = present(&listed(execution_state, None)).state_line();

        for forbidden in ["Working", "Needs You", "Ready"] {
            assert!(
                !line.contains(forbidden),
                "{execution_state} produced {line:?}"
            );
        }
    }
}

/// A value from a newer daemon is unknown, not a fourth behaviour: the wire
/// contract says an unrecognised execution state is read as unknown, and the
/// surface must not quietly disagree.
#[test]
fn an_unrecognised_execution_state_is_shown_as_unknown() {
    let later = present(&listed("suspended", None));

    assert_eq!(later, present(&listed("unknown", None)));
}

/// Exited is the one main state execution truth may establish on its own, and
/// it is stated alone. "Exited · Status unknown" would say the session might
/// still need something, which it cannot.
#[test]
fn an_exited_session_says_nothing_about_status() {
    let line = present(&listed("exited", None)).state_line();

    assert_eq!(line, "Exited");
    assert!(!line.contains("unknown"), "{line}");
}

/// A screen Corral cannot serve is a secondary line and a refusal, and it
/// leaves the main state alone: the process may be running perfectly.
#[test]
fn an_unserveable_screen_is_secondary_and_refuses_open() {
    let presented = present(&listed("running", Some(TerminalAccess::Unavailable)));

    assert_eq!(presented.screen, Some("Screen unavailable"));
    assert_eq!(presented.refuses_open(), Some("Screen unavailable"));
    assert_eq!(
        presented.state_line(),
        "Running · Status unknown",
        "a capability fact leaked into the main state"
    );
}

/// The internal word never reaches a person, whatever the field says.
#[test]
fn nothing_a_surface_renders_says_poisoned() {
    for access in [
        Some(TerminalAccess::Available),
        Some(TerminalAccess::Unavailable),
        None,
    ] {
        let presented = present(&listed("running", access));
        let rendered = format!(
            "{} {}",
            presented.state_line(),
            presented.screen.unwrap_or("")
        );

        for forbidden in ["oison", "Broken", "Error"] {
            assert!(!rendered.contains(forbidden), "{rendered:?}");
        }
    }
}

/// Absence is not a refusal. A daemon that never sent the field, and one that
/// sent a word this build does not know, both leave Open on offer — the
/// answer comes from trying, not from a guess (`AGENTS.md` §Protocol).
#[test]
fn an_unknown_terminal_access_does_not_refuse_open() {
    for access in [Some(TerminalAccess::Available), None] {
        let presented = present(&listed("running", access));

        assert_eq!(presented.screen, None);
        assert_eq!(presented.refuses_open(), None);
    }
}
