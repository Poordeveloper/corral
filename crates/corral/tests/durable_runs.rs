//! The durable lifecycle of a managed run: what `corrald` writes, when, and
//! what it refuses to write twice.
//!
//! These read the registry directly rather than through the wire, because
//! nothing in protocol 1 reports durable facts and the point is what landed in
//! the log — not what a daemon was willing to say about it.

// The repository allows unwrap/expect in tests; that setting does not reach
// helpers that sit outside a `#[test]` function in an integration-test crate.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod support;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use serde_json::{Value, json};
use support::wire::{RawClient, error_code};
use support::{SETTLE, TestAccount, stderr, wait_until};

/// The facts one Session's stream holds, oldest first.
fn kinds(registry: &Path) -> Vec<String> {
    // Deliberately the second opener: reading what the daemon wrote is the
    // whole point, and going through SQLite is how a fact becomes visible.
    #[allow(clippy::disallowed_methods)]
    let connection = rusqlite::Connection::open(registry).expect("open the registry");
    let mut statement = connection
        .prepare("SELECT kind FROM session_events ORDER BY global_seq")
        .expect("prepare");
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query");
    rows.map(|row| row.expect("a kind")).collect()
}

/// Every Run in the store: its id, its end state, and the occurrence time
/// recorded for that end.
fn runs(registry: &Path) -> Vec<(String, Option<String>, Option<i64>)> {
    #[allow(clippy::disallowed_methods)]
    let connection = rusqlite::Connection::open(registry).expect("open the registry");
    let mut statement = connection
        .prepare("SELECT id, end_state, ended_at_ms FROM runs ORDER BY accepted_seq")
        .expect("prepare");
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<i64>>(2)?,
            ))
        })
        .expect("query");
    rows.map(|row| row.expect("a run")).collect()
}

fn bindings(registry: &Path) -> Vec<(String, String, String)> {
    #[allow(clippy::disallowed_methods)]
    let connection = rusqlite::Connection::open(registry).expect("open the registry");
    let mut statement = connection
        .prepare("SELECT kind, provider, provenance FROM bindings")
        .expect("prepare");
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .expect("query");
    rows.map(|row| row.expect("a binding")).collect()
}

fn new_session(client: &mut RawClient, id: u64, command: &str, argv: &[&str]) -> Value {
    client
        .request(
            id,
            "session.new",
            Some(json!({
                "command_id": command,
                "argv": argv,
                "rows": 24,
                "cols": 80,
            })),
        )
        .expect("session.new answered")
}

fn result(frame: &Value) -> &Value {
    frame
        .get("outcome")
        .and_then(|outcome| outcome.get("result"))
        .unwrap_or_else(|| panic!("not a result: {frame}"))
}

fn field(frame: &Value, name: &str) -> String {
    result(frame)
        .get(name)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("no {name} in {frame}"))
        .to_owned()
}

/// The regression this whole design exists for.
///
/// `true` exits before anything downstream of the spawn has finished, so the
/// reaper is holding the exit while the start is still being written. If the
/// order were wrong — a `RunEnded` reaching the store before its `RunStarted` —
/// nothing would fail: `record_run_ended` on an unrecorded Run withholds the
/// fact silently, leaving a durable Run that looks legitimate and stays open
/// forever (grill Q9).
#[test]
fn an_instantly_exiting_command_records_a_start_and_then_an_end() {
    let account = TestAccount::new("instant-exit");
    let _daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());
    client.establish();

    let started = new_session(&mut client, 1, "cmd-1", &["/usr/bin/true"]);
    let run = field(&started, "run_id");

    wait_until(SETTLE, || {
        runs(&account.registry())
            .first()
            .is_some_and(|(_, end, _)| end.is_some())
    });

    let recorded = runs(&account.registry());
    assert_eq!(recorded.len(), 1, "one command, one Run");
    assert_eq!(recorded[0].0, run, "the Run the client was told about");
    assert_eq!(
        recorded[0].1.as_deref(),
        Some("completed"),
        "an exit Corral watched happen"
    );
    assert!(
        kinds(&account.registry()).contains(&"run-started".to_owned()),
        "the start is a durable fact, not something inferred from the end"
    );
}

/// A managed session's Session, its runtime binding, its Run and its receipt
/// are one accepted command. A receipt without its Run would name a Session
/// whose episode nothing can describe.
#[test]
fn opening_a_managed_session_records_the_whole_command_at_once() {
    let account = TestAccount::new("managed-shape");
    let _daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());
    client.establish();

    new_session(&mut client, 1, "cmd-1", &["/bin/sh", "-c", "sleep 30"]);

    let recorded = kinds(&account.registry());
    assert_eq!(
        recorded,
        [
            "session-created",
            "binding-added",
            "run-started",
            "command-accepted"
        ]
    );
    assert_eq!(
        bindings(&account.registry()),
        [(
            "runtime".to_owned(),
            "corral".to_owned(),
            "corral-created".to_owned()
        )],
        "a runtime Corral created is named in the namespace Corral reserves \
         for what it minted (ADR 0008 D1, D3)"
    );
}

/// A lost response makes a client retry. The retry must answer with what the
/// first execution made, and must not start a second agent.
#[test]
fn a_retried_command_starts_one_runtime_and_replays_its_answer() {
    let account = TestAccount::new("retry");
    let _daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());
    client.establish();

    let first = new_session(&mut client, 1, "cmd-1", &["/bin/sh", "-c", "sleep 30"]);
    let again = new_session(&mut client, 2, "cmd-1", &["/bin/sh", "-c", "sleep 30"]);

    assert_eq!(field(&first, "session_id"), field(&again, "session_id"));
    assert_eq!(field(&first, "run_id"), field(&again, "run_id"));
    assert_eq!(runs(&account.registry()).len(), 1, "one command, one Run");
    let listed = client
        .request(3, "session.list", None)
        .expect("the daemon answered");
    assert_eq!(
        listed["outcome"]["result"]["sessions"]
            .as_array()
            .expect("a list")
            .len(),
        1,
        "a retry must not leave a second managed runtime behind"
    );
}

/// The same holds across a connection the client had to reopen, which is what
/// a lost response actually looks like: the daemon no longer remembers the
/// request, and the durable receipt is the replay authority.
#[test]
fn a_retry_on_a_new_connection_replays_from_the_receipt() {
    let account = TestAccount::new("retry-reconnect");
    let _daemon = account.start_daemon();
    let first = {
        let mut client = RawClient::connect(&account.socket());
        client.establish();
        new_session(&mut client, 1, "cmd-1", &["/bin/sh", "-c", "sleep 30"])
    };

    let mut client = RawClient::connect(&account.socket());
    client.establish();
    let again = new_session(&mut client, 1, "cmd-1", &["/bin/sh", "-c", "sleep 30"]);

    assert_eq!(field(&first, "run_id"), field(&again, "run_id"));
    assert_eq!(runs(&account.registry()).len(), 1);
}

/// One command id means one semantic command. A different one under the same
/// id executes nothing — and is told so in a way that says retrying will never
/// help.
#[test]
fn the_same_id_with_a_different_command_is_refused_without_spawning() {
    let account = TestAccount::new("conflict");
    let _daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());
    client.establish();
    new_session(&mut client, 1, "cmd-1", &["/bin/sh", "-c", "sleep 30"]);

    let refused = new_session(&mut client, 2, "cmd-1", &["/bin/sh", "-c", "sleep 60"]);

    assert_eq!(error_code(&refused), Some("command_id_conflict"));
    assert_eq!(runs(&account.registry()).len(), 1, "nothing was executed");
}

/// Attaching and detaching are their own facts. Detaching does not end the
/// Run: closing a surface never terminates managed work.
#[test]
fn attaching_and_detaching_append_their_own_facts() {
    let account = TestAccount::new("attachment");
    let _daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());
    client.establish();
    let started = new_session(&mut client, 1, "cmd-1", &["/bin/sh", "-c", "sleep 30"]);
    let session = field(&started, "session_id");

    let granted = client
        .request(2, "terminal.attach", Some(json!({ "session_id": session })))
        .expect("terminal.attach answered");
    let token = field(&granted, "attach_token");
    {
        let mut channel = RawClient::connect(&account.socket());
        channel.say_hello_with_role(&token);
        wait_until(SETTLE, || {
            kinds(&account.registry()).contains(&"run-attached".to_owned())
        });
    }

    wait_until(SETTLE, || {
        kinds(&account.registry()).contains(&"run-detached".to_owned())
    });
    let recorded = kinds(&account.registry());
    assert_eq!(
        recorded,
        [
            "session-created",
            "binding-added",
            "run-started",
            "command-accepted",
            "run-attached",
            "run-detached"
        ]
    );
    assert!(
        runs(&account.registry())[0].1.is_none(),
        "detaching is not an end"
    );
}

/// A managed runtime does not survive its owning daemon (ADR 0007 L6), so the
/// next daemon closes what the last one left open — as unverifiable, because
/// Corral did not watch it end and may not say that it did.
#[test]
fn a_new_daemon_closes_the_episodes_its_predecessor_left_open() {
    let account = TestAccount::new("reconcile");
    let daemon = account.start_daemon();
    {
        let mut client = RawClient::connect(&account.socket());
        client.establish();
        new_session(&mut client, 1, "cmd-1", &["/bin/sh", "-c", "sleep 30"]);
    }
    daemon.signal(rustix::process::Signal::TERM);
    let (_code, logs) = daemon.wait();
    assert!(
        runs(&account.registry())[0].1.is_none(),
        "the daemon does not reap what it hangs up on the way out: {logs}"
    );

    let _successor = account.start_daemon();

    wait_until(SETTLE, || {
        runs(&account.registry())
            .first()
            .is_some_and(|(_, end, _)| end.is_some())
    });
    let recorded = runs(&account.registry());
    assert_eq!(
        recorded[0].1.as_deref(),
        Some("unverifiable"),
        "a daemon that did not watch a process end never says that it exited"
    );
    assert_eq!(
        recorded[0].2, None,
        "a daemon's startup is not when a process stopped"
    );
    assert!(
        !kinds(&account.registry()).contains(&"run-detached".to_owned()),
        "no detach is invented for a viewer that never detached"
    );
}

/// The whole path a person takes, with the durable record behind it: `corral
/// new` starts a session, attaches, and the run it created is recorded.
#[test]
fn corral_new_records_the_run_it_started() {
    let account = TestAccount::new("cli-new");
    let output = support::run(
        account
            .corral()
            .arg("new")
            .arg("--")
            .arg("/usr/bin/true")
            .stdin(std::process::Stdio::null()),
    );

    assert!(output.status.success(), "{}", stderr(&output));
    wait_until(SETTLE, || {
        runs(&account.registry())
            .first()
            .is_some_and(|(_, end, _)| end.is_some())
    });
    assert_eq!(runs(&account.registry()).len(), 1);
}

/// Two retries arriving at one live daemon are the window the durable receipt
/// cannot close on its own: both can read "no receipt" before either commits.
/// The daemon's in-flight claim is what makes the second one wait (grill Q8).
#[test]
fn concurrent_retries_of_one_command_start_one_runtime() {
    let account = TestAccount::new("concurrent-retry");
    let _daemon = account.start_daemon();
    // Connected and established first, so the race is between two `session.new`
    // requests rather than between two handshakes.
    let mut clients: Vec<RawClient> = (0..2)
        .map(|_| {
            let mut client = RawClient::connect(&account.socket());
            client.establish();
            client
        })
        .collect();
    let second = clients.pop().expect("two clients");
    let first = clients.pop().expect("two clients");

    let answers: Vec<Value> = std::thread::scope(|scope| {
        let racing: Vec<_> = [first, second]
            .into_iter()
            .map(|mut client| {
                scope.spawn(move || {
                    new_session(&mut client, 1, "cmd-1", &["/bin/sh", "-c", "sleep 30"])
                })
            })
            .collect();
        racing
            .into_iter()
            .map(|racer| racer.join().expect("a racer finished"))
            .collect()
    });

    assert_eq!(
        field(&answers[0], "run_id"),
        field(&answers[1], "run_id"),
        "both callers are told about the same Run"
    );
    assert_eq!(
        runs(&account.registry()).len(),
        1,
        "one command id, one runtime"
    );
}

/// A runtime whose Run could not be recorded is not a session. It is hung up
/// and reaped rather than left alive and unlistable, nothing durable is left
/// half-written, and the daemon stops rather than serving from a store it can
/// no longer vouch for (grill Q9, Q14).
///
/// The store is made unwritable by taking write permission off its directory,
/// which SQLite needs for a journal. Reads keep working, so the daemon gets
/// all the way past its pre-spawn consult — which is the window under test.
#[test]
fn a_run_whose_start_cannot_be_recorded_leaves_nothing_behind() {
    let account = TestAccount::new("unwritable");
    let daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());
    client.establish();
    let readable_only = std::fs::Permissions::from_mode(0o555);
    let writable = std::fs::metadata(account.state_dir())
        .expect("the state directory")
        .permissions();
    std::fs::set_permissions(account.state_dir(), readable_only)
        .expect("make the store unwritable");
    // Proves the store is still readable, which is what puts the failure in
    // the window under test: the daemon gets past its vouch and past the
    // pre-spawn receipt consult, and fails on the commit that follows the
    // spawn. Without this the test would also pass if nothing were ever
    // spawned at all.
    assert_eq!(
        client
            .request(1, "session.list", None)
            .expect("the daemon answered")["outcome"]["result"],
        json!({"sessions": []}),
        "reads still work, so the write is what fails"
    );

    let answer = client.request(
        2,
        "session.new",
        Some(json!({
            "command_id": "cmd-1",
            "argv": ["/bin/sh", "-c", "sleep 300"],
            "rows": 24,
            "cols": 80,
        })),
    );

    assert!(
        answer.is_none(),
        "a store that cannot be trusted is not answered from: {answer:?}"
    );
    let (code, logs) = daemon.wait();
    std::fs::set_permissions(account.state_dir(), writable).expect("restore the store");
    assert_eq!(code, Some(1), "{logs}");
    assert_eq!(
        runs(&account.registry()),
        Vec::new(),
        "a start that did not commit is not a Run"
    );
    assert!(
        !kinds(&account.registry()).contains(&"command-accepted".to_owned()),
        "and no receipt claims the command succeeded"
    );
}

/// Detaching after a run has ended is the ordinary shape, not an integrity
/// failure. `RunEnded` is terminal for a Run's attachment state, so the store
/// refuses the fact — and a daemon that read that refusal as "this daemon can
/// no longer account for its runs" would shut down every time somebody was
/// watching when an agent finished (grill Q11).
#[test]
fn detaching_after_a_run_ended_leaves_the_daemon_serving() {
    let account = TestAccount::new("detach-after-end");
    let _daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());
    client.establish();
    let started = new_session(&mut client, 1, "cmd-1", &["/bin/sh", "-c", "sleep 0.2"]);
    let session = field(&started, "session_id");
    let granted = client
        .request(2, "terminal.attach", Some(json!({ "session_id": session })))
        .expect("terminal.attach answered");
    let token = field(&granted, "attach_token");
    let channel = {
        let mut channel = RawClient::connect(&account.socket());
        channel.say_hello_with_role(&token);
        channel
    };
    wait_until(SETTLE, || {
        kinds(&account.registry()).contains(&"run-attached".to_owned())
    });

    // The run ends underneath the person watching it, and only then do they go.
    wait_until(SETTLE, || {
        runs(&account.registry())
            .first()
            .is_some_and(|(_, end, _)| end.is_some())
    });
    drop(channel);

    // Nothing about a detach may end the daemon, whatever the store said about
    // the fact itself.
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(
        client
            .request(3, "session.list", None)
            .expect("the daemon is still serving")["outcome"]["result"]["sessions"]
            .as_array()
            .expect("a list")
            .len(),
        1
    );
}

/// The same fact from the other direction, and the one `corral new -- true`
/// meets every time: a person attaches to a session whose run has already
/// finished, which is exactly what a finished screen is for (ADR 0007 L2).
#[test]
fn attaching_to_a_finished_run_leaves_the_daemon_serving() {
    let account = TestAccount::new("attach-after-end");
    let _daemon = account.start_daemon();
    let mut client = RawClient::connect(&account.socket());
    client.establish();
    let started = new_session(&mut client, 1, "cmd-1", &["/usr/bin/true"]);
    let session = field(&started, "session_id");
    wait_until(SETTLE, || {
        runs(&account.registry())
            .first()
            .is_some_and(|(_, end, _)| end.is_some())
    });

    let granted = client
        .request(2, "terminal.attach", Some(json!({ "session_id": session })))
        .expect("terminal.attach answered");
    let mut channel = RawClient::connect(&account.socket());
    channel.say_hello_with_role(&field(&granted, "attach_token"));
    drop(channel);

    std::thread::sleep(std::time::Duration::from_millis(300));
    assert!(
        client.request(3, "session.list", None).is_some(),
        "attaching to a finished run is not a reason to stop serving"
    );
}
