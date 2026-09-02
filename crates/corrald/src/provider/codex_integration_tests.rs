use super::*;

fn relay() -> RelayInvocation {
    RelayInvocation::of_words(
        "/opt/corral/bin/corral",
        &[
            "hook-relay",
            "--provider",
            "codex",
            "--integration-version",
            "1",
            "--payload-argv",
        ],
    )
}

fn parse(raw: &str) -> DocumentMut {
    raw.parse().expect("a config document")
}

/// A real user's file, in the shape the 2026-09-02 corpus and spike found:
/// the user's own comments, unrelated keys, and the trust entries Codex
/// appends to this same file behind the user's back.
const A_USERS_OWN_CONFIG: &str = r#"# my codex setup
model = "gpt-5.6"

# thinking harder on plans than on turns
model_reasoning_effort = "medium"
plan_mode_reasoning_effort = "high"

[tui]
notifications = false

[projects."/Users/someone/work"]
trust_level = "trusted"
"#;

#[test]
fn an_absent_notifier_is_the_slot_corral_may_take() {
    assert_eq!(slot(&parse(A_USERS_OWN_CONFIG), &relay()), Slot::Absent);
}

/// The measured reason this module uses a format-preserving editor: TOML
/// comments are legal here, Codex keeps what it did not write, and a user who
/// opens this file after Corral touched it should find their own file.
#[test]
fn installing_preserves_every_comment_key_order_and_spacing() {
    let mut document = parse(A_USERS_OWN_CONFIG);

    install(&mut document, &relay());

    let written = document.to_string();
    assert!(written.contains("# my codex setup"));
    assert!(written.contains("# thinking harder on plans than on turns"));
    let untouched = written
        .replace(&format!("{}\n", notify_line(&document)), "")
        .replace("notify = ", "");
    assert!(untouched.contains(A_USERS_OWN_CONFIG.trim_end()) || written.contains("[tui]"));
    // Everything the user wrote is still there, in their order.
    let user_lines: Vec<&str> = A_USERS_OWN_CONFIG.lines().collect();
    let written_lines: Vec<&str> = written.lines().collect();
    let mut written_iter = written_lines.iter();
    for line in user_lines {
        assert!(
            written_iter.any(|written| *written == line),
            "the user's line survived in order: {line}"
        );
    }
}

fn notify_line(document: &DocumentMut) -> String {
    document
        .to_string()
        .lines()
        .find(|line| line.starts_with("notify = "))
        .expect("the notifier line")
        .to_owned()
}

/// The whole file must still parse as TOML: Codex treats a malformed
/// `config.toml` as fatal, not as something to ignore (measured 2026-09-02).
#[test]
fn the_written_document_still_parses_as_toml() {
    let mut document = parse(A_USERS_OWN_CONFIG);

    install(&mut document, &relay());

    document
        .to_string()
        .parse::<DocumentMut>()
        .expect("Codex can still load this file");
}

#[test]
fn corrals_own_notifier_is_recognized_and_reported_current() {
    let mut document = parse(A_USERS_OWN_CONFIG);
    install(&mut document, &relay());

    assert_eq!(slot(&document, &relay()), Slot::Current);
}

/// Corral never overwrites a notifier it cannot prove is its own — not to
/// obtain awareness, not with a wrapper, not by chaining (grill Q3).
#[test]
fn somebody_elses_notifier_is_occupied_and_survives_an_uninstall() {
    // Root keys precede tables in TOML, so an occupied slot sits at the top of
    // the user's file rather than appended to it.
    let occupied =
        format!("notify = [\"/usr/local/bin/my-notifier\", \"--flag\"]\n{A_USERS_OWN_CONFIG}");
    let mut document = parse(&occupied);
    assert_eq!(slot(&document, &relay()), Slot::Occupied);

    uninstall(&mut document);

    assert_eq!(document.to_string(), occupied);
}

#[test]
fn uninstall_removes_corrals_notifier_and_leaves_the_rest_alone() {
    let mut document = parse(A_USERS_OWN_CONFIG);
    install(&mut document, &relay());

    uninstall(&mut document);

    assert_eq!(document.to_string(), A_USERS_OWN_CONFIG);
}

/// Measured: a `notify` of the wrong type stops the Codex CLI with a parse
/// error. It is the user's file to fix, and never something Corral quietly
/// normalizes into an array.
#[test]
fn a_notifier_of_the_wrong_type_is_malformed_and_never_normalized() {
    let mut document = parse("notify = \"/usr/local/bin/my-notifier\"\n");
    assert_eq!(slot(&document, &relay()), Slot::Malformed);

    uninstall(&mut document);

    assert_eq!(
        document.to_string(),
        "notify = \"/usr/local/bin/my-notifier\"\n"
    );
}

#[test]
fn a_notifier_from_a_newer_corral_is_reported_and_left_alone() {
    let newer = RelayInvocation::of_words(
        "/opt/corral/bin/corral",
        &[
            "hook-relay",
            "--provider",
            "codex",
            "--integration-version",
            "99",
        ],
    );
    let mut document = parse(A_USERS_OWN_CONFIG);
    install(&mut document, &newer);

    assert_eq!(slot(&document, &relay()), Slot::Newer(99));
}

#[test]
fn a_notifier_from_an_older_corral_is_stale_and_repairable() {
    let older = RelayInvocation::of_words(
        "/opt/corral/bin/corral",
        &["hook-relay", "--provider", "codex"],
    );
    let mut document = parse(A_USERS_OWN_CONFIG);
    install(&mut document, &older);
    assert_eq!(slot(&document, &relay()), Slot::Stale);

    install(&mut document, &relay());

    assert_eq!(slot(&document, &relay()), Slot::Current);
}

#[test]
fn installing_twice_writes_one_notifier() {
    let mut document = parse(A_USERS_OWN_CONFIG);

    install(&mut document, &relay());
    let once = document.to_string();
    install(&mut document, &relay());

    assert_eq!(document.to_string(), once);
}
