use super::*;

use crate::provider::launch::RelayInvocation;

fn relay() -> RelayInvocation {
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

fn parse(raw: &str) -> Value {
    serde_json::from_str(raw).expect("a settings document")
}

/// The real shape of a user's file, from the 2026-09-02 corpus: third-party
/// hooks on three events, several of them with matchers, plus settings that
/// have nothing to do with hooks.
fn a_users_own_settings() -> Value {
    parse(
        r#"{
            "env": { "USE_BUILTIN_RIPGREP": "1" },
            "permissions": { "allow": ["Bash(ls:*)"], "defaultMode": "plan" },
            "hooks": {
                "Notification": [
                    { "matcher": "permission_prompt",
                      "hooks": [{ "type": "command", "command": "claude-notify permission" }] }
                ],
                "PostToolUse": [
                    { "matcher": "ExitPlanMode",
                      "hooks": [{ "type": "command", "command": "claude-notify plan" }] }
                ]
            },
            "statusLine": { "type": "command", "command": "~/.claude/statusline.ts" },
            "model": "fable"
        }"#,
    )
}

#[test]
fn a_fresh_document_gains_every_event_corral_installs() {
    let mut document = parse("{}");

    install(&mut document, &relay());

    for event in EVENTS {
        let entries: Vec<&Value> = entries_for(&document, event).collect();
        assert_eq!(entries.len(), 1, "{event} carries exactly Corral's entry");
        assert!(is_corrals(entries[0]));
    }
}

/// The command Claude runs is guarded, so a Corral binary that is gone is
/// silent instead of printing an error on every prompt and every turn
/// (ADR 0013 D8, measured 2026-09-02).
#[test]
fn every_installed_command_fails_open() {
    let mut document = parse("{}");

    install(&mut document, &relay());

    for event in EVENTS {
        let entry = entries_for(&document, event).next().expect("an entry");
        let command = commands_of(entry).next().expect("a command");
        assert!(
            command.ends_with("|| true"),
            "{event} carries a fail-open guard: {command}"
        );
    }
}

/// Nothing Corral writes into this file may be a place a payload could land.
/// The command is Corral's static invocation and Corral's own quoted words.
#[test]
fn the_installed_command_carries_no_shell_syntax_beyond_the_guard() {
    let mut document = parse("{}");

    install(&mut document, &relay());

    let entry = entries_for(&document, "SessionStart")
        .next()
        .expect("an entry");
    let command = commands_of(entry).next().expect("a command");
    let without_guard = command.strip_suffix(" || true").expect("the guard");
    assert!(!without_guard.contains("&&"));
    assert!(!without_guard.contains(';'));
    assert!(!without_guard.contains('$'));
    assert!(!without_guard.contains('|'));
}

/// The measured hazard this whole module exists for: a comment makes Claude
/// reject the entire settings file, silently dropping every setting in it.
#[test]
fn nothing_written_is_a_comment_and_the_result_is_strict_json() {
    let mut document = a_users_own_settings();

    install(&mut document, &relay());

    let written = serde_json::to_string_pretty(&document).expect("serialize");
    assert!(!written.contains("//"));
    assert!(!written.contains("/*"));
    serde_json::from_str::<Value>(&written).expect("strict JSON round-trips");
}

#[test]
fn a_third_partys_hooks_survive_an_install_untouched() {
    let before = a_users_own_settings();
    let mut document = before.clone();

    install(&mut document, &relay());

    assert_eq!(document.get("env"), before.get("env"));
    assert_eq!(document.get("permissions"), before.get("permissions"));
    assert_eq!(document.get("statusLine"), before.get("statusLine"));
    assert_eq!(document.get("model"), before.get("model"));
    // An event Corral does not install is not touched at all.
    assert_eq!(
        document["hooks"]["PostToolUse"],
        before["hooks"]["PostToolUse"]
    );
    // An event Corral does install keeps the user's entry beside Corral's.
    let notifications: Vec<&Value> = entries_for(&document, "Notification").collect();
    assert_eq!(notifications.len(), 2);
    assert_eq!(notifications[0], &before["hooks"]["Notification"][0]);
    assert!(is_corrals(notifications[1]));
}

#[test]
fn installing_twice_leaves_one_corral_entry_per_event() {
    let mut document = a_users_own_settings();

    install(&mut document, &relay());
    let once = document.clone();
    install(&mut document, &relay());

    assert_eq!(document, once);
}

#[test]
fn uninstall_removes_corrals_entries_and_leaves_the_users_file_as_it_was() {
    let before = a_users_own_settings();
    let mut document = before.clone();
    install(&mut document, &relay());

    uninstall(&mut document);

    assert_eq!(document, before);
}

/// Corral created the containers on a file that had none, so uninstall takes
/// them away again rather than leaving `"hooks": {}` behind.
#[test]
fn uninstall_leaves_no_empty_container_corral_created() {
    let mut document = parse("{}");
    install(&mut document, &relay());

    uninstall(&mut document);

    assert_eq!(document, parse("{}"));
}

#[test]
fn a_document_without_corrals_entries_reports_absent() {
    assert_eq!(
        installed(&a_users_own_settings(), &relay()),
        Installed::Absent
    );
}

#[test]
fn a_document_this_binary_just_wrote_reports_current() {
    let mut document = a_users_own_settings();
    install(&mut document, &relay());

    assert_eq!(installed(&document, &relay()), Installed::Current);
}

/// Half an installation is stale, not absent: repair brings it forward rather
/// than treating the file as untouched.
#[test]
fn a_partly_installed_document_reports_stale() {
    let mut document = a_users_own_settings();
    install(&mut document, &relay());
    document["hooks"]
        .as_object_mut()
        .expect("hooks")
        .remove("Stop");

    assert_eq!(installed(&document, &relay()), Installed::Stale);
}

/// An older Corral never rewrites what a newer Corral wrote (ADR 0013 D2).
#[test]
fn an_entry_from_a_newer_corral_is_reported_and_not_claimed_as_current() {
    let newer = RelayInvocation::of_words(
        "/opt/corral/bin/corral",
        &[
            "hook-relay",
            "--provider",
            "claude",
            "--integration-version",
            "99",
        ],
    );
    let mut document = a_users_own_settings();
    install(&mut document, &newer);

    assert_eq!(installed(&document, &relay()), Installed::Newer(99));
}

/// A version this binary understands is repairable in place, which is what
/// makes the discriminant worth writing.
#[test]
fn an_entry_from_an_older_corral_is_stale_and_repair_brings_it_forward() {
    let older = RelayInvocation::of_words(
        "/opt/corral/bin/corral",
        &["hook-relay", "--provider", "claude"],
    );
    let mut document = a_users_own_settings();
    install(&mut document, &older);
    assert_eq!(installed(&document, &relay()), Installed::Stale);

    install(&mut document, &relay());

    assert_eq!(installed(&document, &relay()), Installed::Current);
}
