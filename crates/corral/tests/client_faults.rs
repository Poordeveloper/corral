//! What a surface does when the daemon it reached is not one it can use.
//!
//! These run against a stand-in daemon: the behaviours under test only appear
//! opposite a peer the real daemon would never be.

// The repository allows unwrap/expect in tests; that setting does not reach
// helpers that sit outside a `#[test]` function in an integration-test crate.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod support;

use std::time::Duration;

use support::wire::{hello_reply, spawn_fake_daemon};
use support::{TestAccount, run, stderr};

#[test]
fn an_incompatible_daemon_is_terminal_and_is_left_running() {
    let account = TestAccount::new("incompatible").with_activation_deadline(Duration::from_secs(3));
    let fake = spawn_fake_daemon(&account.socket(), Some(hello_reply(9, 9, "incompatible")));

    let output = run(account.corral().arg("ping"));

    let message = stderr(&output);
    assert!(!output.status.success());
    assert!(message.contains("protocol 9"), "{message}");
    assert!(message.contains("protocol 1"), "{message}");
    assert_eq!(
        fake.connections(),
        1,
        "an incompatible daemon is not retried into compatibility"
    );
    assert!(
        std::os::unix::net::UnixStream::connect(account.socket()).is_ok(),
        "a client has no authority to stop the daemon it found"
    );
}

/// Both peers run the same symmetric predicate, so a daemon claiming they agree
/// when the versions say otherwise is an internal bug, not an authority.
#[test]
fn a_daemons_own_verdict_is_never_taken_on_trust() {
    let account = TestAccount::new("divergent").with_activation_deadline(Duration::from_secs(3));
    let _fake = spawn_fake_daemon(&account.socket(), Some(hello_reply(9, 9, "compatible")));

    let output = run(account.corral().arg("ping"));

    let message = stderr(&output);
    assert!(!output.status.success());
    assert!(message.contains("disagreed"), "{message}");
}

#[test]
fn a_malformed_server_hello_fails_the_handshake() {
    let account = TestAccount::new("bad-hello").with_activation_deadline(Duration::from_secs(3));
    let _fake = spawn_fake_daemon(
        &account.socket(),
        Some(b"{\"type\":\"response\",\"id\":0,\"outcome\":{\"result\":{}}}\n".to_vec()),
    );

    let output = run(account.corral().arg("ping"));

    let message = stderr(&output);
    assert!(!output.status.success());
    assert!(message.contains("malformed"), "{message}");
}

/// A daemon lost mid-request is an honest failure. Nothing is replayed:
/// protocol 1 defines no idempotency for the commands that would need it.
#[test]
fn a_daemon_that_vanishes_mid_request_is_reported_not_replayed() {
    let account = TestAccount::new("vanishing").with_activation_deadline(Duration::from_secs(3));
    let fake = spawn_fake_daemon(&account.socket(), Some(hello_reply(1, 1, "compatible")));

    let output = run(account.corral().arg("ping"));

    let message = stderr(&output);
    assert!(!output.status.success());
    assert!(message.contains("closed the connection"), "{message}");
    assert!(message.contains("not retried"), "{message}");
    assert_eq!(fake.connections(), 1);
}
