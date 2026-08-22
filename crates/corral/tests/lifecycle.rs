//! Daemon lifetime: who keeps it alive, what ends it, and what survives.

// The repository allows unwrap/expect in tests; that setting does not reach
// helpers that sit outside a `#[test]` function in an integration-test crate.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod support;

use std::time::{Duration, Instant};

use rustix::process::Signal;
use support::wire::RawClient;
use support::{SETTLE, TestAccount, lock_is_held, run, stderr, wait_until};

#[test]
fn an_idle_daemon_exits_and_the_next_client_starts_a_fresh_one() {
    let account = TestAccount::new("idle-exit").with_idle_grace(Duration::from_millis(300));

    let first = run(account.corral().arg("ping"));
    assert!(first.status.success(), "{}", stderr(&first));

    wait_until(SETTLE, || !lock_is_held(&account.lock()));
    assert!(
        !account.socket().exists(),
        "a clean exit removes its own rendezvous"
    );

    let second = run(account.corral().arg("ping"));
    assert!(second.status.success(), "{}", stderr(&second));
}

/// A connection that never says hello has no claim on the daemon's life;
/// otherwise repeatedly connecting would keep an idle daemon alive forever.
#[test]
fn pending_connections_never_keep_an_idle_daemon_alive() {
    let account = TestAccount::new("pending-starvation")
        .with_idle_grace(Duration::from_millis(500))
        .with_pre_hello_deadline(Duration::from_secs(30));
    let daemon = account.start_daemon();

    let mut pending = Vec::new();
    let keep_knocking_until = Instant::now() + Duration::from_secs(2);
    while Instant::now() < keep_knocking_until {
        // Once the daemon has exited there is nothing to connect to, which is
        // the point of the test.
        if let Some(client) = RawClient::try_connect(&account.socket()) {
            pending.push(client);
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    assert!(pending.len() > 1, "the test never actually connected");
    wait_until(SETTLE, || !lock_is_held(&account.lock()));
    let (code, log) = daemon.wait();
    assert_eq!(code, Some(0), "{log}");
    drop(pending);
}

/// An established client holds the daemon open, and lets go when it leaves.
#[test]
fn an_established_client_holds_the_daemon_open() {
    let account = TestAccount::new("established-hold").with_idle_grace(Duration::from_millis(300));
    let daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());
    client.establish();

    std::thread::sleep(Duration::from_millis(900));
    assert!(lock_is_held(&account.lock()), "the daemon exited too early");

    drop(client);
    wait_until(SETTLE, || !lock_is_held(&account.lock()));
    let (code, log) = daemon.wait();
    assert_eq!(code, Some(0), "{log}");
}

#[test]
fn a_signal_closes_established_clients_and_releases_the_claim() {
    let account = TestAccount::new("sigterm");
    let daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());
    client.establish();

    daemon.signal(Signal::TERM);

    // The in-flight surface fails honestly rather than being replayed.
    assert!(
        client.receive().is_none(),
        "a shutting-down daemon closes its established connections"
    );
    wait_until(SETTLE, || !lock_is_held(&account.lock()));
    let (code, log) = daemon.wait();
    assert_eq!(code, Some(0), "{log}");
    assert!(!lock_is_held(&account.lock()));
    assert!(!account.socket().exists());
}

/// Losing the singleton race is the ordinary outcome of a cold-start stampede,
/// not a failure, and the loser must touch nothing on its way out.
#[test]
fn a_daemon_that_loses_the_race_exits_without_disturbing_the_winner() {
    // The winner must not idle out while the loser is still waiting for the
    // claim, or the test would be measuring an idle exit instead of a race.
    let account = TestAccount::new("race-loser").with_idle_grace(Duration::from_secs(30));
    let _winner = account.start_daemon();

    let loser = run(&mut account.corrald());

    assert!(loser.status.success(), "{}", stderr(&loser));
    assert!(account.socket().exists(), "the winner still serves");
    let output = run(account.corral().arg("ping"));
    assert!(output.status.success(), "{}", stderr(&output));
}

/// An abrupt death leaves the rendezvous behind. The kernel releases the claim,
/// and the next claim winner owns the residue.
#[test]
fn crash_residue_is_owned_by_the_next_claim_winner() {
    let account = TestAccount::new("crash-residue");
    let daemon = account.start_daemon();

    daemon.signal(Signal::KILL);
    let (_code, _log) = daemon.wait();

    wait_until(SETTLE, || !lock_is_held(&account.lock()));
    assert!(
        account.socket().exists(),
        "a killed daemon cannot clean up after itself"
    );

    let output = run(account.corral().arg("ping"));
    assert!(output.status.success(), "{}", stderr(&output));
}

/// PR1 owns no durable Corral state: a fresh daemon reconstructs nothing.
#[test]
fn a_restarted_daemon_reports_the_same_empty_world() {
    let account = TestAccount::new("restart-state").with_idle_grace(Duration::from_millis(300));

    let before = run(account.corral().arg("list"));
    wait_until(SETTLE, || !lock_is_held(&account.lock()));
    let after = run(account.corral().arg("list"));

    assert!(before.status.success(), "{}", stderr(&before));
    assert!(after.status.success(), "{}", stderr(&after));
    assert_eq!(support::stdout(&before), support::stdout(&after));
}

// The commit-then-establish order is covered by `corrald::lifecycle` unit
// tests rather than here. There is no externally observable window: the daemon
// commits, closes its connections and exits in one step, so a process-level
// test cannot distinguish "the guard refused the hello" from "the process was
// gone". A test that cannot fail for the intended reason is worse than none,
// and the one that used to live here was proved vacuous by removing the guard
// and watching it stay green.
