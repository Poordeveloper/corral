//! The relay's poverty, asserted.
//!
//! What `corral hook-relay` must never do is the whole contract: never write,
//! never fail, never start a daemon, never wait indefinitely. Claude Code
//! reads hook stdout and a nonzero exit as decisions, so every one of these is
//! a way the relay could steer the user's agent (ADR 0004 D1, D4).
//!
//! The 50 ms budget itself is measured evidence recorded with the change, not
//! a timing assertion repeated on every run: a per-run deadline that tight is
//! a flake generator on a loaded machine, and the flake law owns that trade
//! (`AGENTS.md` §Tests). What is asserted here is the behaviour — silence,
//! success, no activation — under a hard limit generous enough to mean
//! "bounded" rather than "fast".

// The repository allows unwrap/expect in tests; that setting does not reach
// helpers that sit outside a `#[test]` function in an integration-test crate.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod support;

use std::io::{Read, Write};
use std::process::Stdio;
use std::time::{Duration, Instant};

use support::{TestAccount, lock_is_held};

/// A limit that means "this returned rather than hung". Far above the
/// interference budget on purpose.
const BOUNDED: Duration = Duration::from_secs(5);

const PAYLOAD: &str = r#"{"session_id":"a","hook_event_name":"Stop"}"#;

/// Run the relay against this account, with a payload on standard input.
fn relay(account: &TestAccount, payload: &str) -> (std::process::Output, Duration) {
    let mut child = account
        .corral()
        .arg("hook-relay")
        .arg("--provider")
        .arg("claude")
        .arg("--token")
        .arg("0123456789abcdef0123456789abcdef")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the relay");
    let started = Instant::now();
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(payload.as_bytes())
        .expect("write the payload");
    let output = child.wait_with_output().expect("the relay returned");
    (output, started.elapsed())
}

/// Nothing was written, the exit was 0, and it returned rather than hung.
fn failed_open(output: &std::process::Output, elapsed: Duration, what: &str) {
    assert!(output.status.success(), "{what}: {:?}", output.status);
    assert!(
        output.stdout.is_empty(),
        "{what}: the relay wrote to stdout: {:?}",
        String::from_utf8_lossy(&output.stdout),
    );
    assert!(
        output.stderr.is_empty(),
        "{what}: the relay wrote to stderr: {:?}",
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(elapsed < BOUNDED, "{what}: took {elapsed:?}");
}

/// The ordinary case a person will hit most: no daemon running at all.
///
/// An absent socket means `corrald` is not running, which means fail open now
/// — and it must never be the thing that starts one. A shim that could
/// activate the daemon is a shim that can delay the user's agent by however
/// long a cold start takes (ADR 0004 D1).
#[test]
fn with_no_daemon_the_relay_is_silent_and_starts_nothing() {
    let account = TestAccount::new("relay-no-daemon");

    let (output, elapsed) = relay(&account, PAYLOAD);

    failed_open(&output, elapsed, "no daemon");
    assert!(!account.socket().exists(), "the relay created a rendezvous");
    assert!(!lock_is_held(&account.lock()), "the relay started a daemon");
    // The strongest form of the same assertion: activation would have had to
    // create the run directory to claim anything in it.
    assert!(
        !account.corral_root().join("run").exists(),
        "the relay built a rendezvous it must never touch",
    );
}

/// A definite error, answered now rather than by waiting the budget out.
#[test]
fn a_refused_connection_is_silent_and_starts_nothing() {
    let account = TestAccount::new("relay-refused");
    support::create_private_dir_all(&account.corral_root().join("run"));
    // A pathname that is a socket nothing listens on: bound, then dropped.
    let hook = account.corral_root().join("run/hook.sock");
    drop(std::os::unix::net::UnixListener::bind(&hook).expect("bind"));
    std::fs::remove_file(&hook).expect("unlink");
    std::fs::write(&hook, "not a socket").expect("a file in its place");

    let (output, elapsed) = relay(&account, PAYLOAD);

    failed_open(&output, elapsed, "refused");
    assert!(!lock_is_held(&account.lock()), "the relay started a daemon");
}

/// A listener that accepts and never answers is the shape a daemon under load
/// has. The relay gives up on its own deadline and returns control to the
/// provider; the budget is never widened to mask daemon slowness.
#[test]
fn a_listener_that_never_answers_does_not_hold_the_relay() {
    let account = TestAccount::new("relay-slow");
    support::create_private_dir_all(&account.corral_root().join("run"));
    let hook = account.corral_root().join("run/hook.sock");
    let listener = std::os::unix::net::UnixListener::bind(&hook).expect("bind");
    let accepting = std::thread::spawn(move || {
        // Accepted and held: read the delivery, answer nothing, and keep the
        // connection open until this thread ends with the test.
        if let Ok((mut stream, _)) = listener.accept() {
            let mut sink = [0_u8; 1024];
            let _ = stream.read(&mut sink);
            std::thread::sleep(Duration::from_secs(2));
        }
    });

    let (output, elapsed) = relay(&account, PAYLOAD);

    failed_open(&output, elapsed, "slow ack");
    let _ = accepting.join();
}

/// A payload the relay cannot carry is a definite error it fails open on, not
/// something it repairs, reports, or re-encodes.
#[test]
fn an_unusable_payload_is_still_silent_success() {
    let account = TestAccount::new("relay-unusable");

    let (output, elapsed) = relay(&account, "");

    failed_open(&output, elapsed, "empty payload");
}

/// A departed daemon leaves no endpoint behind.
///
/// The pathname is the daemon's to create and to remove, and a shutdown that
/// left it would mean anything reading the path sees a daemon that is gone as
/// one that is present.
#[test]
fn a_departing_daemon_takes_its_hook_endpoint_with_it() {
    let account = TestAccount::new("relay-endpoint-gone");
    let hook = account.corral_root().join("run/hook.sock");
    let daemon = account.start_daemon();
    support::wait_until(Duration::from_secs(10), || hook.exists());

    daemon.signal(rustix::process::Signal::TERM);
    let (_status, _log) = daemon.wait();
    support::wait_until(Duration::from_secs(10), || !lock_is_held(&account.lock()));

    assert!(!hook.exists(), "the hook endpoint outlived its daemon");
}

/// Delivery that works, end to end, over the daemon's real endpoint: the relay
/// still says nothing and still exits 0, because the outcome is not a shim's
/// business either way.
#[test]
fn a_delivered_event_is_answered_and_still_silent() {
    let account = TestAccount::new("relay-delivers");
    let daemon = account.start_daemon();

    let (output, elapsed) = relay(&account, PAYLOAD);

    failed_open(&output, elapsed, "delivered");
    drop(daemon);
}

/// The other delivery shape, and the same poverty.
///
/// Codex appends its payload as one final argument and writes nothing to
/// standard input (ADR 0009 D2), so this invocation must complete without
/// anything ever arriving there. Standard input is left open and empty on
/// purpose: a relay that consulted it would be waiting on a pipe the provider
/// never opened, once per event.
///
/// That the bytes reach the daemon intact is proven end to end in
/// `managed_codex`, over a real launch token; what this owns is the contract
/// every path here owns — silence, success, and no activation.
#[test]
fn an_argv_payload_invocation_needs_nothing_on_standard_input() {
    let account = TestAccount::new("relay-argv");
    let payload =
        r#"{"type":"agent-turn-complete","thread-id":"01a0576f-0ecc-7b21-9719-f38f9e4ef933"}"#;

    let mut child = account
        .corral()
        .arg("hook-relay")
        .arg("--provider")
        .arg("codex")
        .arg("--token")
        .arg("0123456789abcdef0123456789abcdef")
        .arg("--payload-argv")
        .arg(payload)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the relay");
    let started = Instant::now();
    // Held open and never written to for as long as the relay runs: the pipe
    // outlives the child only because this handle is dropped after it exits.
    let held = child.stdin.take().expect("stdin");
    let output = child.wait_with_output().expect("the relay returned");
    drop(held);

    failed_open(&output, started.elapsed(), "argv payload");
    assert!(!lock_is_held(&account.lock()), "the relay started a daemon");
}
