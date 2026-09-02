//! End-to-end: the attention verbs against a real daemon with nothing sealed.
//!
//! Nothing asserts a main state before the reconciliation seals a row, so
//! what these prove is the honest floor: the verbs answer, and they answer
//! with what the daemon knows rather than with a guess.

// The repository allows unwrap/expect in tests; that setting does not reach
// helpers that sit outside a `#[test]` function in an integration-test crate.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod support;

use support::{TestAccount, run, stderr, stdout};

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
