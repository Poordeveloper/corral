use super::*;

use corral_protocol::method::TerminalAccess;

fn listed(execution_state: &str, terminal_access: Option<TerminalAccess>) -> SessionListItem {
    SessionListItem {
        session_id: "0f9b6c1a-7d2e-4a55-9c31-000000000000".to_owned(),
        title: "sh".to_owned(),
        execution_state: execution_state.to_owned(),
        terminal_access,
    }
}

/// One projection, asserted from both of its callers. Two surfaces describing
/// the same session differently would be worse than either being wrong alone
/// (grill Q2), and nothing but a test keeps them from drifting.
#[test]
fn the_cli_says_what_the_session_list_says_about_the_same_session() {
    for execution_state in ["running", "exited", "unknown", "from-a-later-build"] {
        for access in [
            Some(TerminalAccess::Available),
            Some(TerminalAccess::Unavailable),
            None,
        ] {
            let item = listed(execution_state, access);
            let printed = session_rows(&item).join("\n");

            // The first line of a row is the identity and the title, which the
            // two surfaces lay out differently on purpose. Everything after it
            // is the state text, and that must be identical.
            for said in corral_tui::row_text(&item).into_iter().skip(1) {
                assert!(
                    printed.contains(&said),
                    "{execution_state}/{access:?}: the list says {said:?}, the CLI printed \
                     {printed:?}"
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
