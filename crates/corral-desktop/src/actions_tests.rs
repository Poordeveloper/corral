use super::*;

fn form(directory: &str, arguments: &str) -> NewSessionForm {
    NewSessionForm {
        provider: Provider::Codex,
        working_directory: directory.to_owned(),
        arguments: arguments.to_owned(),
    }
}

fn a_directory() -> String {
    std::env::temp_dir().to_string_lossy().into_owned()
}

/// Round 2, Q8: the form can only ever ask for a provider. There is no arm
/// for a command, and the arguments are the provider's own, split as the TUI
/// splits them.
#[test]
fn the_form_asks_for_a_provider_and_never_a_command() {
    let launch = form(&a_directory(), "--model o3 --  x")
        .preflight()
        .expect("a directory that exists");

    assert_eq!(
        launch.requested,
        Requested::Provider {
            name: "codex".to_owned(),
            args: vec![
                "--model".to_owned(),
                "o3".to_owned(),
                "--".to_owned(),
                "x".to_owned()
            ],
        }
    );
    assert_eq!(
        launch.site.working_directory.as_deref(),
        Some(Path::new(&a_directory()))
    );
    assert_eq!((launch.site.rows, launch.site.cols), (None, None));
}

#[test]
fn the_providers_are_the_two_corral_composes_a_command_for() {
    assert_eq!(Provider::ClaudeCode.wire_name(), "claude");
    assert_eq!(Provider::Codex.wire_name(), "codex");
    assert_eq!(Provider::ALL.len(), 2);
}

/// Only what this surface can see for itself is checked in advance; the
/// provider grammar stays the daemon's.
#[test]
fn the_preflight_refuses_what_it_can_see_and_nothing_else() {
    assert_eq!(
        form("   ", "").preflight().unwrap_err(),
        Preflight::WorkingDirectoryMissing
    );
    assert_eq!(
        form("relative/path", "").preflight().unwrap_err(),
        Preflight::WorkingDirectoryRelative("relative/path".to_owned())
    );
    assert_eq!(
        form("/definitely/not/here/corral-desktop-test", "")
            .preflight()
            .unwrap_err(),
        Preflight::WorkingDirectoryNotFound("/definitely/not/here/corral-desktop-test".to_owned())
    );
    assert!(
        form(&a_directory(), "--anything --the-daemon --decides")
            .preflight()
            .is_ok()
    );
}

#[test]
fn actions_are_offered_by_capability() {
    let none = Offered::by(Capabilities::default());
    assert_eq!(none, Offered::default());

    let all = Offered::by(Capabilities {
        managed_sessions: true,
        attention: true,
        ..Capabilities::default()
    });
    assert_eq!(
        all,
        Offered {
            new_session: true,
            continue_in_corral: true,
            acknowledge: true
        }
    );
}
