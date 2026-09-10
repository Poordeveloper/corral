//! End-to-end: the attention verbs against a real daemon with nothing sealed.
//!
//! Nothing asserts a main state before the reconciliation seals a row, so
//! what these prove is the honest floor: the verbs answer, and they answer
//! with what the daemon knows rather than with a guess.

// The repository allows unwrap/expect in tests; that setting does not reach
// helpers that sit outside a `#[test]` function in an integration-test crate.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod support;

use std::time::Duration;

use support::provider::{Script, new_claude, permission_dialog, session_start};
use support::{SETTLE, TestAccount, run, stderr, stdout, wait_until};

#[test]
fn nothing_needs_you_on_a_daemon_with_no_sessions() {
    let account = TestAccount::new("needs-empty");

    let output = run(account.corral().arg("needs"));

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output).trim(), "Nothing needs you.");
}

#[test]
fn the_report_is_empty_before_anything_transitioned() {
    let account = TestAccount::new("report-empty");

    let output = run(account.corral().args(["attention", "report"]));

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output).trim(), "No attention journal days.");
}

/// A session with no current item has nothing to acknowledge, and the
/// command says so rather than acknowledging a future one.
#[test]
fn acknowledging_a_session_without_an_item_says_so() {
    let account = TestAccount::new("ack-nothing");
    let started = run(account
        .corral()
        .args(["new", "--", "sh", "-c", "sleep 30"])
        .stdin(std::process::Stdio::null()));
    let session = stderr(&started)
        .lines()
        .find_map(|line| line.strip_prefix("session "))
        .map(str::trim)
        .map(str::to_owned);
    let Some(session) = session else {
        // `corral new` attaches, and a null stdin detaches it at once; the
        // session id is on stderr either way.
        panic!("no session id in: {}", stderr(&started));
    };

    let output = run(account.corral().args(["ack", &session]));

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output).trim(), "Nothing to acknowledge.");
}

/// Activity is entitled by construction — bytes on a PTY Corral owns are the
/// agent drawing, whatever version it is — so a managed session that keeps
/// drawing reads Working with nothing sealed, and Exited once it ends.
#[test]
fn a_drawing_session_reads_working_and_an_ended_one_exited() {
    let account = TestAccount::new("working-from-activity");
    let started = run(account
        .corral()
        .args([
            "new",
            "--",
            "sh",
            "-c",
            "for i in $(seq 1 40); do echo tick; sleep 0.2; done",
        ])
        .stdin(std::process::Stdio::null()));
    let session = stderr(&started)
        .lines()
        .find_map(|line| line.strip_prefix("session "))
        .map(str::trim)
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("no session id in: {}", stderr(&started)));

    let listed = || stdout(&run(account.corral().arg("list")));
    support::wait_until(support::SETTLE, || listed().contains("Working"));
    support::wait_until(support::SETTLE * 3, || listed().contains("Exited"));

    // The journal saw the transitions, and says so per day.
    let report = stdout(&run(account.corral().args(["attention", "report"])));
    assert!(report.contains("day"), "{report}");
    assert!(!report.contains("INCOMPLETE"), "{report}");
    let _ = session;
}

/// A false-item dispute binds one item. With none current there is nothing
/// to bind it to, the command refuses before sending, and the journal holds
/// no record of a statement nobody could make (completion grill Q15).
#[test]
fn disputing_a_session_without_an_item_refuses_and_records_nothing() {
    let account = TestAccount::new("dispute-nothing");
    let started = run(account
        .corral()
        .args(["new", "--", "sh", "-c", "sleep 30"])
        .stdin(std::process::Stdio::null()));
    let session = stderr(&started)
        .lines()
        .find_map(|line| line.strip_prefix("session "))
        .map(str::trim)
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("no session id in: {}", stderr(&started)));

    let output = run(account.corral().args(["attention", "dispute", &session]));

    assert!(!output.status.success(), "{}", stdout(&output));
    assert_eq!(
        stderr(&output).trim(),
        "No current attention item. If Corral failed to surface an item, use --missed."
    );
    let report = stdout(&run(account.corral().args(["attention", "report"])));
    for day in report.lines().filter(|line| line.starts_with("20")) {
        let columns: Vec<&str> = day.split_whitespace().collect();
        // day, transitions, needs you (all), trusted, ready, false, missed
        assert_eq!(columns[5], "0", "no dispute was recorded: {day}");
        assert_eq!(columns[6], "0", "no dispute was recorded: {day}");
    }
}

/// A missed item is its own statement: due, and not surfaced. It names no
/// item, so a session with none current records it, and the report counts
/// it apart from false items.
#[test]
fn a_missed_item_is_recorded_without_an_item_and_counted_apart() {
    let account = TestAccount::new("dispute-missed");
    let started = run(account
        .corral()
        .args(["new", "--", "sh", "-c", "sleep 30"])
        .stdin(std::process::Stdio::null()));
    let session = stderr(&started)
        .lines()
        .find_map(|line| line.strip_prefix("session "))
        .map(str::trim)
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("no session id in: {}", stderr(&started)));

    let output = run(account.corral().args([
        "attention",
        "dispute",
        &session,
        "--missed",
        "--note",
        "the approval prompt showed no row",
    ]));

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output).trim(), "Recorded.");
    let report = stdout(&run(account.corral().args(["attention", "report"])));
    let today = report
        .lines()
        .find(|line| line.starts_with("20"))
        .unwrap_or_else(|| panic!("a day in: {report}"));
    let columns: Vec<&str> = today.split_whitespace().collect();
    // day, transitions, needs you (all), trusted, ready, false, missed
    assert_eq!(columns[5], "0", "{today}");
    assert_eq!(columns[6], "1", "{today}");
    assert!(!today.contains("INCOMPLETE"), "{today}");
}

/// Acknowledging clears the badge, not the item: an acknowledged Needs You is
/// still current and still the one that may be wrong, so a false-item
/// dispute after `ack` names it and is recorded. The item is a real one — a
/// sealed Claude 2.1.258 permission dialog drawn on the daemon's own PTY.
#[test]
fn a_false_item_dispute_still_names_the_item_after_it_was_acknowledged() {
    const FIRST: &str = "33333333-3333-4333-8333-333333333333";
    let account = TestAccount::new("dispute-after-ack")
        .with_mock_provider("claude")
        .with_versioned_claude("2.1.258")
        .with_idle_grace(Duration::from_secs(30));
    let script = Script::new(&account, "dispute-after-ack")
        .holding()
        .fires(&session_start(FIRST, "startup"))
        .fires(&permission_dialog());
    let daemon = account.start_daemon_with(&script.environment());
    let session = new_claude(&account);

    let listed = || stdout(&run(account.corral().arg("list")));
    wait_until(SETTLE, || listed().contains("Needs You"));
    assert!(listed().contains("Needs You"), "{}", listed());

    let acked = run(account.corral().args(["ack", &session]));
    assert!(acked.status.success(), "{}", stderr(&acked));
    assert_eq!(stdout(&acked).trim(), "Acknowledged.");

    let disputed = run(account.corral().args([
        "attention",
        "dispute",
        &session,
        "--note",
        "nothing was asked",
    ]));
    assert!(disputed.status.success(), "{}", stderr(&disputed));
    assert_eq!(stdout(&disputed).trim(), "Recorded.");
    let report = stdout(&run(account.corral().args(["attention", "report"])));
    let today = report
        .lines()
        .find(|line| line.starts_with("20"))
        .unwrap_or_else(|| panic!("a day in: {report}"));
    let columns: Vec<&str> = today.split_whitespace().collect();
    // day, transitions, needs you (all), trusted, ready, false, missed
    assert_eq!(columns[5], "1", "{today}");
    assert_eq!(columns[6], "0", "{today}");
    assert_eq!(columns[3], "1", "a sealed, attested Needs You: {today}");

    drop(daemon);
}
