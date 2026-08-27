use super::*;

use corral_protocol::method::TerminalAccess;

fn listed(execution_state: &str, terminal_access: Option<TerminalAccess>) -> SessionListItem {
    SessionListItem {
        session_id: "0f9b6c1a-7d2e-4a55-9c31-000000000000".to_owned(),
        title: "sh".to_owned(),
        execution_state: execution_state.to_owned(),
        terminal_access,
        provider: None,
        agent_event: None,
    }
}

/// A session the daemon reported provider facts about.
fn reported(kind: &str) -> SessionListItem {
    SessionListItem {
        title: "claude".to_owned(),
        provider: Some(corral_protocol::method::ProviderFacts {
            name: "claude".to_owned(),
            external_id: Some("d2dfcafd-9a73-4162-aa70-dddf99aa6e75".to_owned()),
        }),
        agent_event: Some(corral_protocol::method::AgentEvent {
            kind: corral_protocol::method::AgentEventKind::from_wire(kind),
            at_ms: 1_700_000_000_000,
        }),
        ..listed("running", Some(TerminalAccess::Available))
    }
}

/// One projection, and this surface says what it says. The two surfaces lay a
/// row out differently on purpose; what they must not do is describe the same
/// session differently, which would be worse than either being wrong alone
/// (grill Q2).
///
/// Asserted against the projection rather than against the list's own layout,
/// because the projection is what both of them read.
#[test]
fn the_cli_says_what_the_projection_says_about_a_session() {
    for execution_state in ["running", "exited", "unknown", "from-a-later-build"] {
        for access in [
            Some(TerminalAccess::Available),
            Some(TerminalAccess::Unavailable),
            None,
        ] {
            let item = listed(execution_state, access);
            let presented = corral_tui::present(&item);
            let printed = session_rows(&item).join("\n");

            let mut said = vec![presented.state_line()];
            said.extend(presented.beneath());
            for line in said {
                assert!(
                    printed.contains(&line),
                    "{execution_state}/{access:?}: the projection says {line:?}, the CLI \
                     printed {printed:?}"
                );
            }
        }
    }
}

/// The capability line is printed when there is one and never invented when
/// there is not — including for a value this build could not read.
#[test]
fn the_cli_mentions_an_unserveable_screen_only_when_the_daemon_said_so() {
    let unavailable =
        session_rows(&listed("running", Some(TerminalAccess::Unavailable))).join("\n");
    assert!(unavailable.contains("Screen unavailable"), "{unavailable}");

    for access in [Some(TerminalAccess::Available), None] {
        let printed = session_rows(&listed("running", access)).join("\n");

        assert!(!printed.contains("Screen unavailable"), "{printed}");
        assert_eq!(session_rows(&listed("running", access)).len(), 1);
    }
}

/// A person reads an id here and types it into `attach`, so the two have to
/// agree on how much of it is enough.
#[test]
fn the_printed_id_is_a_prefix_attach_resolves() {
    let item = listed("running", Some(TerminalAccess::Available));

    let printed = session_rows(&item).join("\n");

    assert!(printed.starts_with("0f9b6c1a"), "{printed}");
    assert!(item.session_id.starts_with("0f9b6c1a"));
    assert!(printed.contains("sh"), "{printed}");
}

/// The two surfaces lay a row out differently and say the same words about the
/// same session. A provider fact is the newest thing they could disagree
/// about, so it is asserted the same way (grill Q2).
#[test]
fn the_cli_says_what_the_projection_says_about_a_reported_fact() {
    for kind in [
        "session_started",
        "turn_started",
        "turn_ended",
        "awaiting_input",
        "session_ended",
        "a_kind_from_later",
    ] {
        let item = reported(kind);
        let presented = corral_tui::present(&item);
        let printed = session_rows(&item).join("\n");

        for line in presented.beneath() {
            assert!(
                printed.contains(&line),
                "{kind}: {line:?} is missing from {printed:?}"
            );
        }
        // A kind this build cannot name renders nothing, and the raw provider
        // spelling never reaches a person.
        assert!(!printed.contains("a_kind_from_later"), "{printed}");
    }
}

/// The provider session id is not a display fact. It rides the wire so the
/// daemon can act on it; a list read at a glance is not where an opaque
/// identifier belongs.
#[test]
fn the_cli_does_not_print_the_provider_session_id() {
    let item = reported("turn_ended");

    let printed = session_rows(&item).join("\n");

    assert!(!printed.contains("d2dfcafd"), "{printed}");
}
