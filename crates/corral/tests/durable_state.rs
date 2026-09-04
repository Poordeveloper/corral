//! Fail-closed: a daemon never serves state-derived claims from a store it
//! cannot vouch for, before readiness or after it (ADR 0002, Q14).

// The repository allows unwrap/expect in tests; that setting does not reach
// helpers that sit outside a `#[test]` function in an integration-test crate.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod support;

use std::process::Stdio;

use support::wire::RawClient;
use support::{TestAccount, create_private_dir_all, run, stderr, stdout};

/// A `corrald` started directly, run to completion, and asked what it did.
fn start_and_wait(account: &TestAccount) -> (Option<i32>, String) {
    let child = account
        .corrald()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start corrald");
    let output = child.wait_with_output().expect("wait for corrald");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// The state directory has to be usable before anything else happens, so a
/// daemon that cannot have one never binds its endpoint.
#[test]
fn an_unusable_state_directory_prevents_readiness() {
    let account = TestAccount::new("state-dir");
    std::fs::write(account.state_dir(), b"not a directory").expect("occupy the state directory");

    let (code, logs) = start_and_wait(&account);

    assert_eq!(code, Some(1), "{logs}");
    assert!(!account.socket().exists(), "the endpoint was never bound");
}

/// A file that is not a Corral store is a startup failure, not an empty
/// registry: an empty list is a claim, and this store cannot support one.
#[test]
fn a_store_that_is_not_a_store_prevents_readiness() {
    let account = TestAccount::new("corrupt");
    create_private_dir_all(&account.state_dir());
    std::fs::write(account.registry(), b"this is not a database").expect("write");

    let (code, logs) = start_and_wait(&account);

    assert_eq!(code, Some(1), "{logs}");
    assert!(!account.socket().exists(), "the endpoint was never bound");
}

/// A cold start creates the registry before the daemon answers anything, so no
/// client can be told the daemon is ready and then find out otherwise.
#[test]
fn a_ready_daemon_has_already_opened_its_registry() {
    let account = TestAccount::new("ready");

    let output = run(account.corral().arg("ping"));

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(account.registry().exists());
    assert!(stdout(&output).contains("protocol"));
}

/// The one behaviour the zero-wire exception was granted for: a transient
/// refusal is answered rather than dropping the caller, and the connection
/// stays usable (`docs/decisions/2026-08-23-pr2-transient-state-error-code.md`).
///
/// Holding the store costs the daemon its whole busy timeout by construction,
/// so this test is deliberately one of the slow ones.
#[test]
fn a_registry_held_by_another_writer_is_answered_not_dropped() {
    let account = TestAccount::new("held");
    let _daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());
    client.establish();

    // Deliberately the second opener: something else holding the store is the
    // condition under test.
    #[allow(clippy::disallowed_methods)]
    let holder = rusqlite::Connection::open(account.registry()).expect("open the registry");
    holder
        .execute_batch("BEGIN EXCLUSIVE")
        .expect("hold the registry");
    let answer = client
        .request(1, "session.list", None)
        .expect("the daemon answered rather than closing");
    holder
        .execute_batch("COMMIT")
        .expect("release the registry");

    assert_eq!(
        answer["outcome"]["error"]["code"], "busy",
        "expected a retryable answer, got {answer}"
    );
    let after = client
        .request(2, "session.list", None)
        .expect("the connection is still usable");
    assert_eq!(
        after["outcome"]["result"],
        serde_json::json!({"sessions": []})
    );
}

/// The same for the continuation preflight, which reads the registry to
/// decide and is therefore reachable by the same held lock. Every error from
/// the decision used to end the daemon, so one other writer holding the store
/// — a backup tool — could stop every session's control plane by way of a
/// question about one session (`corral-state`: `Busy` is the canonical
/// transient condition).
#[test]
fn a_continuation_asked_while_the_registry_is_held_is_answered_not_dropped() {
    let account = TestAccount::new("held-continuation");
    let _daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());
    client.establish();

    #[allow(clippy::disallowed_methods)]
    let holder = rusqlite::Connection::open(account.registry()).expect("open the registry");
    holder
        .execute_batch("BEGIN EXCLUSIVE")
        .expect("hold the registry");
    let answer = client
        .request(
            1,
            "session.continuation",
            Some(serde_json::json!({
                "session_id": "0f9b6c1a-4444-4444-8444-000000000004",
            })),
        )
        .expect("the daemon answered rather than closing");
    holder
        .execute_batch("COMMIT")
        .expect("release the registry");

    assert_eq!(
        answer["outcome"]["error"]["code"], "busy",
        "expected a retryable answer, got {answer}"
    );
    // The daemon is still there, which is the half of this that the old
    // behaviour took away.
    let after = client
        .request(2, "session.list", None)
        .expect("the connection is still usable");
    assert_eq!(
        after["outcome"]["result"],
        serde_json::json!({"sessions": []})
    );
}

/// Once the store stops being the one the daemon validated, the daemon stops
/// serving rather than answering from it. The caller sees the connection go,
/// never a normal-looking empty list.
#[test]
fn a_store_replaced_underneath_the_daemon_stops_it_serving() {
    let account = TestAccount::new("replaced");
    let daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());
    client.establish();
    let before = client
        .request(1, "session.list", None)
        .expect("the daemon answered");
    assert_eq!(
        before["outcome"]["result"],
        serde_json::json!({"sessions": []})
    );

    // Another process rewriting the store's identity is an invariant
    // violation: every fact the daemon has read from it is now suspect.
    // Deliberately the second opener: the daemon is holding this store, and
    // something else writing to it is the condition under test.
    #[allow(clippy::disallowed_methods)]
    let connection = rusqlite::Connection::open(account.registry()).expect("open the registry");
    connection
        .execute(
            "UPDATE node_identity SET node_id = '00000000-0000-4000-8000-000000000000'",
            [],
        )
        .expect("rewrite the store identity");
    drop(connection);

    let after = client.request(2, "session.list", None);

    assert!(
        after.is_none(),
        "expected the connection to end, got {after:?}"
    );
    let (code, logs) = daemon.wait();
    assert_eq!(code, Some(1), "{logs}");
    assert!(
        logs.contains("no longer be trusted"),
        "the daemon says why it stopped: {logs}"
    );
}
