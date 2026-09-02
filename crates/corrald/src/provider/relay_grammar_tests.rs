use super::*;

use crate::provider::launch::RelayInvocation;

fn corral_relay() -> RelayInvocation {
    RelayInvocation::of_words(
        "/opt/corral/bin/corral",
        &[
            "hook-relay",
            "--provider",
            "claude",
            "--integration-version",
            "1",
        ],
    )
}

#[test]
fn corrals_own_shell_command_is_recognized_as_corrals() {
    let command = format!("{} || true", corral_relay().shell_command());

    assert!(invokes_corral_relay(&command));
    assert_eq!(declared_version(&command), Some(1));
}

/// The guard is not ownership evidence. A third party's fail-open hook must
/// survive an install, a repair, and an uninstall untouched (grill Q1′).
#[test]
fn a_third_partys_fail_open_hook_is_not_corrals() {
    assert!(!invokes_corral_relay(
        "/usr/local/bin/notify-me stop || true"
    ));
}

/// A program whose name merely contains Corral's is a different program.
#[test]
fn a_similarly_named_program_is_not_corrals() {
    assert!(!invokes_corral_relay(
        "'/usr/bin/corral-helper' 'hook-relay'"
    ));
    assert!(!invokes_corral_relay("'/usr/bin/corral'"));
}

/// The subcommand carries the claim. Corral's own binary doing something else
/// is not an integration entry.
#[test]
fn corral_invoked_for_anything_else_is_not_an_integration_entry() {
    assert!(!invokes_corral_relay("'/opt/corral/bin/corral' 'list'"));
}

#[test]
fn an_entry_written_before_the_discriminant_declares_no_version() {
    let command = RelayInvocation::of_words(
        "/opt/corral/bin/corral",
        &["hook-relay", "--provider", "claude"],
    )
    .shell_command();

    assert!(invokes_corral_relay(&command));
    assert_eq!(declared_version(&command), None);
}

/// A path a shell needs quoting for round-trips: the recognizer reads back
/// exactly what the writer produced.
#[test]
fn a_path_that_needs_quoting_survives_the_round_trip() {
    let relay = RelayInvocation::of_words(
        "/home/someone's dir/corral",
        &["hook-relay", "--provider", "codex"],
    );

    assert!(invokes_corral_relay(&relay.shell_command()));
}

/// Codex's `notify` is an array of words the document already split, so
/// recognition there never goes through a shell at all.
#[test]
fn codex_argv_words_are_recognized_without_a_shell() {
    let relay = RelayInvocation::of_words(
        "/opt/corral/bin/corral",
        &[
            "hook-relay",
            "--provider",
            "codex",
            "--integration-version",
            "1",
            "--payload-argv",
        ],
    );
    let words: Vec<String> = relay.words().map(str::to_owned).collect();

    assert!(words_invoke_corral_relay(&words));
    assert_eq!(version_in(&words), Some(1));
}

#[test]
fn an_empty_command_claims_nothing() {
    assert!(!invokes_corral_relay(""));
    assert!(!words_invoke_corral_relay(&[]));
}
