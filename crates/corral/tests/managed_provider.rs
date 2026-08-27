//! End-to-end: a managed provider session, from launch to identity to
//! continuation.
//!
//! Every part of the path is the real one — the daemon composes the launch,
//! writes the Corral-owned settings file, spawns through the runtime, and the
//! stand-in provider runs the injected hook command, which is the real
//! `corral hook-relay`, which delivers over the real hook endpoint. What is
//! substituted is the agent, and only the agent: no test calls a real provider
//! (`AGENTS.md` §Tests).

// The repository allows unwrap/expect in tests; that setting does not reach
// helpers that sit outside a `#[test]` function in an integration-test crate.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod support;

use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};
use support::provider::{
    self, Script, agent_event_kind, external_id, launch_files, listed, provider_name, session_end,
    session_start,
};
use support::wire::{RawClient, error_code};
use support::{DaemonProcess, SETTLE, TestAccount, wait_until};

const FIRST: &str = "11111111-1111-4111-8111-111111111111";
const SECOND: &str = "22222222-2222-4222-8222-222222222222";

/// Long enough that a daemon holding no live session does not idle out while a
/// test is still asking it questions.
const GRACE: Duration = Duration::from_secs(30);

/// An account whose `claude` is the scripted stand-in.
fn account(name: &str) -> TestAccount {
    TestAccount::new(name)
        .with_mock_provider("claude")
        .with_idle_grace(GRACE)
}

/// Start the daemon that will spawn the scripted provider.
fn daemon_running(account: &TestAccount, script: &Script) -> DaemonProcess {
    account.start_daemon_with(&script.environment())
}

/// `corral new claude`, and the Session id it printed.
///
/// No terminal on standard input, so the attach loop reads EOF and returns;
/// what is under test is everything before that.
fn new_claude(account: &TestAccount) -> String {
    let output = support::run(
        account
            .corral()
            .arg("new")
            .arg("claude")
            .stdin(std::process::Stdio::null()),
    );
    let stderr = support::stderr(&output);
    assert!(output.status.success(), "{stderr}");
    stderr
        .lines()
        .find_map(|line| line.strip_prefix("session "))
        .unwrap_or_else(|| panic!("a session id in {stderr:?}"))
        .to_owned()
}

fn sessions(client: &mut RawClient, id: u64) -> Vec<Value> {
    let answer = client
        .request(id, "session.list", None)
        .expect("session.list answered");
    answer["outcome"]["result"]["sessions"]
        .as_array()
        .expect("a list")
        .clone()
}

/// The Runs of the only Session the log holds, oldest first.
fn recorded_runs(registry: &Path) -> Vec<(String, Option<String>)> {
    // Reading what the daemon wrote is the point, and going through SQLite is
    // how a fact becomes visible.
    #[allow(clippy::disallowed_methods)]
    let connection = rusqlite::Connection::open(registry).expect("open the registry");
    let mut statement = connection
        .prepare("SELECT id, end_state FROM runs ORDER BY accepted_seq")
        .expect("prepare");
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .expect("query");
    rows.map(|row| row.expect("a run")).collect()
}

/// Every durable fact the log holds, oldest first.
fn recorded_kinds(registry: &Path) -> Vec<String> {
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

/// A launch, its `SessionStart`, and the Attested binding that follows: the
/// whole of what makes a managed session's provider identity real.
#[test]
fn a_managed_launch_learns_its_provider_identity_from_its_own_hooks() {
    let account = account("attested");
    let script = Script::new(&account, "attested")
        .holding()
        .fires(&session_start(FIRST, "startup"));
    let daemon = daemon_running(&account, &script);

    let session = new_claude(&account);

    let mut client = RawClient::connect(&account.socket());
    client.establish();
    // The identity travels the real path — stand-in, relay, endpoint,
    // ingestion, store — so the wait is for delivery rather than for a clock.
    wait_until(provider::DELIVERED, || {
        listed(&sessions(&mut client, 1), &session).and_then(external_id) == Some(FIRST)
    });

    let all = sessions(&mut client, 2);
    let row = listed(&all, &session).expect("the session that reported");
    assert_eq!(provider_name(row), Some("claude"));
    assert_eq!(agent_event_kind(row), Some("session_started"));
    // The main state is untouched. Provider evidence is secondary, past tense,
    // and never a semantic claim (ADR 0004 D7).
    assert_eq!(row["execution_state"], "running");

    // The launch carried Corral's own settings file, and the stand-in found a
    // runnable hook command inside it — which is the only reason the identity
    // above arrived at all.
    let launches = script.launches();
    assert_eq!(launches.len(), 1, "{launches:?}");
    assert!(launches[0].starts_with("--settings "), "{launches:?}");

    // Exactly one Attested provider-session binding, added once.
    let kinds = recorded_kinds(&account.registry());
    assert_eq!(
        kinds.iter().filter(|kind| *kind == "binding-added").count(),
        2,
        "the managed runtime binding and the provider identity: {kinds:?}",
    );
    assert!(
        !kinds.iter().any(|kind| kind == "binding-contested"),
        "{kinds:?}"
    );

    drop(daemon);
}

/// The same identity twice is one binding. A duplicate `SessionStart` — the
/// ordinary shape of a resume — confirms rather than creating a second edge.
#[test]
fn the_same_identity_reported_twice_stays_one_binding() {
    let account = account("idempotent");
    let script = Script::new(&account, "idempotent")
        .holding()
        .fires(&session_start(FIRST, "startup"))
        .fires(&session_start(FIRST, "startup"));
    let daemon = daemon_running(&account, &script);

    let session = new_claude(&account);
    let mut client = RawClient::connect(&account.socket());
    client.establish();
    wait_until(provider::DELIVERED, || {
        recorded_kinds(&account.registry())
            .iter()
            .any(|kind| kind == "binding-confirmed")
    });

    let kinds = recorded_kinds(&account.registry());
    assert_eq!(
        kinds.iter().filter(|kind| *kind == "binding-added").count(),
        2,
        "{kinds:?}",
    );
    assert_eq!(
        listed(&sessions(&mut client, 1), &session).and_then(external_id),
        Some(FIRST),
    );

    drop(daemon);
}

/// The last thing a session says still counts.
///
/// A provider fires its closing hook and then exits, so the delivery and the
/// Run's ending are milliseconds apart. Retiring the launch token off the
/// ingestion path would race that delivery and lose the tail of every session
/// — which is why the ending rides the same queue as the events it follows.
#[test]
fn the_event_a_session_ends_on_still_lands() {
    let account = account("closing-event");
    let script = Script::new(&account, "closing-event")
        .fires(&session_start(FIRST, "startup"))
        .fires(&session_end(FIRST));
    let daemon = daemon_running(&account, &script);
    let session = new_claude(&account);

    let mut client = RawClient::connect(&account.socket());
    client.establish();
    // Its Run is over by now, which is the point: the fact arrived before the
    // ending and must survive it.
    wait_until(SETTLE, || {
        listed(&sessions(&mut client, 1), &session)
            .is_some_and(|listed| listed["execution_state"] == "exited")
    });
    wait_until(provider::DELIVERED, || {
        listed(&sessions(&mut client, 2), &session).and_then(agent_event_kind)
            == Some("session_ended")
    });

    drop(daemon);
}

/// A launch token names one launch, and an event carrying one this daemon
/// never minted is dropped with diagnostics — never correlated by cwd or time
/// (ADR 0004 D5).
#[test]
fn an_event_bearing_a_token_this_daemon_never_minted_binds_nothing() {
    let account = account("stranger-token");
    let script = Script::new(&account, "stranger")
        .holding()
        .fires(&session_start(FIRST, "startup"));
    let daemon = daemon_running(&account, &script);
    let session = new_claude(&account);

    let mut client = RawClient::connect(&account.socket());
    client.establish();
    wait_until(provider::DELIVERED, || {
        listed(&sessions(&mut client, 1), &session).and_then(external_id) == Some(FIRST)
    });

    // A second identity, delivered over a token nobody minted, and a delivery
    // stating a contract this build does not speak.
    deliver_by_hand(
        &account,
        &json!({
            "hook_protocol_version": 1,
            "launch_token": "ffffffffffffffffffffffffffffffff",
            "provider": "claude",
            "shim_version": "0.0.0",
            "payload": session_start(SECOND, "startup").to_string(),
        }),
    );
    deliver_by_hand(
        &account,
        &json!({
            "hook_protocol_version": 99,
            "launch_token": "ffffffffffffffffffffffffffffffff",
            "provider": "claude",
            "shim_version": "0.0.0",
            "payload": session_start(SECOND, "startup").to_string(),
        }),
    );

    // Nothing moved: no session claims the stranger's identity, the one that
    // reported keeps the identity it earned, and nothing was contested.
    std::thread::sleep(Duration::from_millis(300));
    let all = sessions(&mut client, 2);
    assert!(
        all.iter().all(|listed| external_id(listed) != Some(SECOND)),
        "{all:#?}",
    );
    assert_eq!(listed(&all, &session).and_then(external_id), Some(FIRST));
    assert!(
        !recorded_kinds(&account.registry())
            .iter()
            .any(|kind| kind == "binding-contested"),
    );

    drop(daemon);
}

/// The conflict this phase makes durable, and what it revokes.
///
/// The scenario is the one the version matrix observed first-party: one
/// process, one launch token, and a second `SessionStart` naming a different
/// conversation.
#[test]
fn a_contradicting_identity_report_refuses_the_continuation() {
    let account = account("contested");
    let script = Script::new(&account, "contested")
        .holding()
        .fires(&session_start(FIRST, "startup"))
        .fires(&session_end(FIRST))
        .fires(&session_start(SECOND, "resume"))
        .fires(&provider::stop(SECOND));
    let daemon = daemon_running(&account, &script);
    let session = new_claude(&account);

    let mut client = RawClient::connect(&account.socket());
    client.establish();
    // The contest is observable as the withdrawal of the current claim: the
    // identity Corral stood behind is no longer assertable. Waited for through
    // the *last* scripted event, so what follows reads a settled session
    // rather than one still being told things.
    wait_until(provider::DELIVERED, || {
        listed(&sessions(&mut client, 1), &session)
            .and_then(agent_event_kind)
            .map(str::to_owned)
            == Some("turn_ended".to_owned())
    });

    let all = sessions(&mut client, 2);
    let row = listed(&all, &session).expect("the contested session");
    // Withdraw exactly the claim that became unsafe, and nothing else. The
    // conflicting id is never promoted into a replacement (ADR 0004 D8).
    assert_eq!(external_id(row), None);
    assert_eq!(
        provider_name(row),
        Some("claude"),
        "the product is still known"
    );
    assert_eq!(
        agent_event_kind(row),
        Some("turn_ended"),
        "facts belonging to the managed runtime keep arriving",
    );
    // Emitted once, whichever identity later reports name.
    assert_eq!(
        recorded_kinds(&account.registry())
            .iter()
            .filter(|kind| *kind == "binding-contested")
            .count(),
        1,
    );
    // No binding to the conflicting identity was created.
    assert_eq!(
        recorded_kinds(&account.registry())
            .iter()
            .filter(|kind| *kind == "binding-added")
            .count(),
        2,
    );

    // Continuing is refused, and refused for the reason that will not go away.
    let refusal = client
        .request(
            3,
            "session.resume",
            Some(json!({"command_id": "resume-1", "session_id": session})),
        )
        .expect("session.resume answered");
    assert_eq!(error_code(&refusal), Some("invalid_params"), "{refusal}");
    let message = refused_with(&refusal);
    assert!(
        message.contains("contradicts the one Corral accepted"),
        "{message}",
    );
    // No provider external id reaches a resume argv, and nothing was spawned.
    assert_eq!(script.launches().len(), 1, "{:?}", script.launches());

    // Open still works: it rides the Deterministic runtime binding, which a
    // contest does not touch (founder emphasis on R2 Q2).
    let granted = client
        .request(4, "terminal.attach", Some(json!({"session_id": session})))
        .expect("terminal.attach answered");
    assert!(
        granted["outcome"]["result"]["attach_token"].is_string(),
        "a contest disabled an operation it does not govern: {granted}",
    );

    drop(daemon);
}

/// Contested is monotonic, and the report that tests it is the one that looks
/// harmless: the original id, arriving again.
///
/// A person clears a conversation (a second identity, so a contest), then
/// reopens the first one from the in-session picker. Corral must not take that
/// as permission to stand behind the first id again — ADR 0004 D8 says later
/// reports of the original id do not restore it, and clearing a contest needs
/// a correction mechanism this phase does not have.
#[test]
fn the_original_identity_reported_again_does_not_undo_a_contest() {
    let account = account("contested-restored");
    let script = Script::new(&account, "contested-restored")
        .holding()
        .fires(&session_start(FIRST, "startup"))
        .fires(&session_start(SECOND, "clear"))
        .fires(&session_start(FIRST, "resume"))
        .fires(&provider::stop(FIRST));
    let daemon = daemon_running(&account, &script);
    let session = new_claude(&account);

    let mut client = RawClient::connect(&account.socket());
    client.establish();
    // Waited for through the last scripted event, so what follows reads a
    // settled session rather than one still being told things.
    wait_until(provider::DELIVERED, || {
        listed(&sessions(&mut client, 1), &session).and_then(agent_event_kind) == Some("turn_ended")
    });

    let all = sessions(&mut client, 2);
    let row = listed(&all, &session).expect("the contested session");
    assert_eq!(
        external_id(row),
        None,
        "the withdrawn claim came back: {row}",
    );
    assert_eq!(provider_name(row), Some("claude"));

    let kinds = recorded_kinds(&account.registry());
    assert_eq!(
        kinds
            .iter()
            .filter(|kind| *kind == "binding-contested")
            .count(),
        1,
        "{kinds:?}",
    );
    assert!(
        !kinds.iter().any(|kind| kind == "binding-confirmed"),
        "a contested binding was confirmed again: {kinds:?}",
    );

    let refusal = client
        .request(
            3,
            "session.resume",
            Some(json!({"command_id": "resume-1", "session_id": session})),
        )
        .expect("session.resume answered");
    assert!(
        refused_with(&refusal).contains("contradicts the one Corral accepted"),
        "{refusal}",
    );

    drop(daemon);
}

/// The regression that names the ruling. A contest is durable, so a restart
/// does not hand the next continuation an identity Corral already knows is
/// disputed (grill Q3).
#[test]
fn a_contest_still_refuses_a_continuation_after_a_restart() {
    let account = account("contested-restart");
    let script = Script::new(&account, "contested-restart")
        .fires(&session_start(FIRST, "startup"))
        .fires(&session_start(SECOND, "resume"));
    let daemon = daemon_running(&account, &script);
    let session = new_claude(&account);

    wait_until(provider::DELIVERED, || {
        recorded_kinds(&account.registry())
            .iter()
            .any(|kind| kind == "binding-contested")
    });
    // The Run ends cleanly, so the refusal below can only be about identity.
    wait_until(SETTLE, || {
        recorded_runs(&account.registry())
            .first()
            .is_some_and(|(_, end)| end.as_deref() == Some("completed"))
    });

    // A whole new daemon: every launch token is forgotten, and the live
    // evidence every row was carrying is gone with it.
    daemon.signal(rustix::process::Signal::TERM);
    let (_status, _log) = daemon.wait();
    wait_until(SETTLE, || !support::lock_is_held(&account.lock()));
    let restarted = daemon_running(&account, &script);

    let mut client = RawClient::connect(&account.socket());
    client.establish();
    let refusal = client
        .request(
            1,
            "session.resume",
            Some(json!({"command_id": "resume-after-restart", "session_id": session})),
        )
        .expect("session.resume answered");

    assert_eq!(error_code(&refusal), Some("invalid_params"), "{refusal}");
    let message = refused_with(&refusal);
    assert!(
        message.contains("contradicts the one Corral accepted"),
        "a restart must not clear a contest: {message}",
    );
    assert_eq!(script.launches().len(), 1, "nothing was spawned");

    drop(restarted);
}

/// The same Session, a new Run, and the provider's own session id in the
/// resume argv — which is the fact the whole identity path exists to earn.
#[test]
fn continuing_a_session_runs_it_again_under_the_same_identity() {
    let account = account("continue");
    let script = Script::new(&account, "continue").fires(&session_start(FIRST, "startup"));
    let daemon = daemon_running(&account, &script);
    let session = new_claude(&account);

    let mut client = RawClient::connect(&account.socket());
    client.establish();
    // Its identity has to be learned, and its first Run has to be over, before
    // a continuation is eligible.
    wait_until(provider::DELIVERED, || {
        listed(&sessions(&mut client, 1), &session).is_some_and(|listed| {
            external_id(listed) == Some(FIRST) && listed["execution_state"] == "exited"
        })
    });

    let answer = client
        .request(
            2,
            "session.resume",
            Some(json!({"command_id": "resume-1", "session_id": session})),
        )
        .expect("session.resume answered");
    let result = &answer["outcome"]["result"];
    assert_eq!(result["session_id"], session.as_str(), "{answer}");
    let resumed_run = result["run_id"].as_str().expect("a run id").to_owned();

    // The provider was told to continue the conversation Corral recorded, with
    // a fresh injected settings file beside it.
    wait_until(SETTLE, || script.launches().len() == 2);
    let launches = script.launches();
    assert!(
        launches[1].starts_with(&format!("--resume {FIRST} --settings ")),
        "{launches:?}",
    );
    assert_ne!(
        launches[0], launches[1],
        "a resume gets its own settings file"
    );

    // Same Session, a second Run, one managed runtime binding.
    wait_until(SETTLE, || recorded_runs(&account.registry()).len() == 2);
    let runs = recorded_runs(&account.registry());
    assert!(
        runs.iter().any(|(id, _)| *id == resumed_run),
        "{runs:?} does not hold {resumed_run}",
    );
    let kinds = recorded_kinds(&account.registry());
    assert_eq!(
        kinds.iter().filter(|kind| *kind == "binding-added").count(),
        2,
        "a continuation reuses the managed runtime binding: {kinds:?}",
    );
    assert!(
        kinds.iter().any(|kind| kind == "binding-confirmed"),
        "the re-observed identity is confirmed: {kinds:?}",
    );

    // Retried under the same id, it replays the Run it made rather than
    // starting a second.
    let replayed = client
        .request(
            3,
            "session.resume",
            Some(json!({"command_id": "resume-1", "session_id": session})),
        )
        .expect("session.resume answered");
    assert_eq!(
        replayed["outcome"]["result"]["run_id"],
        resumed_run.as_str()
    );
    assert_eq!(script.launches().len(), 2, "a replay starts nothing");

    // The same id carrying a different Session is a different command.
    let conflict = client
        .request(
            4,
            "session.resume",
            Some(json!({
                "command_id": "resume-1",
                "session_id": "00000000-0000-4000-8000-000000000000",
            })),
        )
        .expect("session.resume answered");
    assert_eq!(
        error_code(&conflict),
        Some("command_id_conflict"),
        "{conflict}",
    );

    drop(daemon);
}

/// No override of any kind. A Session still running is refused with the fact
/// stated, because two live executions could otherwise drive one provider
/// conversation — which the version matrix observed a provider permit
/// (grill Q7).
#[test]
fn continuing_a_still_running_session_is_refused() {
    let account = account("continue-live");
    let script = Script::new(&account, "continue-live")
        .holding()
        .fires(&session_start(FIRST, "startup"));
    let daemon = daemon_running(&account, &script);
    let session = new_claude(&account);

    let mut client = RawClient::connect(&account.socket());
    client.establish();
    wait_until(provider::DELIVERED, || {
        listed(&sessions(&mut client, 1), &session).and_then(external_id) == Some(FIRST)
    });

    let refusal = client
        .request(
            2,
            "session.resume",
            Some(json!({"command_id": "resume-1", "session_id": session})),
        )
        .expect("session.resume answered");

    assert_eq!(error_code(&refusal), Some("invalid_params"), "{refusal}");
    assert!(
        refused_with(&refusal).contains("still running"),
        "{refusal}"
    );
    assert_eq!(script.launches().len(), 1, "nothing was spawned");

    drop(daemon);
}

/// A Run whose end Corral could not establish is not a Run Corral may build a
/// continuation on. The refusal states the fact, and there is no way past it
/// in M1.
#[test]
fn continuing_after_an_unverifiable_end_is_refused_with_the_fact_stated() {
    let account = account("continue-unverifiable");
    let script = Script::new(&account, "continue-unverifiable")
        .holding()
        .fires(&session_start(FIRST, "startup"));
    let daemon = daemon_running(&account, &script);
    let session = new_claude(&account);
    wait_until(provider::DELIVERED, || {
        recorded_kinds(&account.registry())
            .iter()
            .filter(|kind| *kind == "binding-added")
            .count()
            == 2
    });

    // The daemon goes without watching its child end, so the Run is
    // reconciled as unverifiable — never as exited (ADR 0007 L6).
    daemon.signal(rustix::process::Signal::TERM);
    let (_status, _log) = daemon.wait();
    wait_until(SETTLE, || !support::lock_is_held(&account.lock()));
    let restarted = daemon_running(&account, &script);
    wait_until(SETTLE, || {
        recorded_runs(&account.registry())
            .first()
            .is_some_and(|(_, end)| end.as_deref() == Some("unverifiable"))
    });

    let mut client = RawClient::connect(&account.socket());
    client.establish();
    let refusal = client
        .request(
            1,
            "session.resume",
            Some(json!({"command_id": "resume-1", "session_id": session})),
        )
        .expect("session.resume answered");

    assert_eq!(error_code(&refusal), Some("invalid_params"), "{refusal}");
    let message = refused_with(&refusal);
    assert!(
        message.contains("cannot verify that the previous run has exited"),
        "{message}",
    );
    assert_eq!(script.launches().len(), 1, "nothing was spawned");

    drop(restarted);
}

/// A Session Corral never learned an identity for has nothing to continue, and
/// says so rather than composing an argv out of nothing.
#[test]
fn continuing_a_session_with_no_provider_identity_is_refused() {
    let account = TestAccount::new("continue-raw").with_idle_grace(GRACE);
    let daemon = account.start_daemon();

    let mut client = RawClient::connect(&account.socket());
    client.establish();
    let started = client
        .request(
            1,
            "session.new",
            Some(json!({"command_id": "cmd-1", "argv": ["/usr/bin/true"]})),
        )
        .expect("session.new answered");
    let session = started["outcome"]["result"]["session_id"]
        .as_str()
        .expect("a session id")
        .to_owned();

    let refusal = client
        .request(
            2,
            "session.resume",
            Some(json!({"command_id": "resume-1", "session_id": session})),
        )
        .expect("session.resume answered");

    assert_eq!(error_code(&refusal), Some("invalid_params"), "{refusal}");
    assert!(
        refused_with(&refusal).contains("has not learned which provider session this is"),
        "{refusal}",
    );

    drop(daemon);
}

/// Established exit: the file goes. That is the only ownership evidence strong
/// enough to destroy it (ADR 0004 D6).
#[test]
fn a_finished_runs_injected_settings_are_removed() {
    let account = account("launch-file");
    let script = Script::new(&account, "launch-file").fires(&session_start(FIRST, "startup"));
    let daemon = daemon_running(&account, &script);

    new_claude(&account);

    wait_until(SETTLE, || launch_files(&account).is_empty());

    drop(daemon);
}

/// An unverifiable owner keeps its file. Losing Corral's ownership is not
/// proof the provider process is dead, and a restart is exactly the moment
/// that distinction is easiest to get wrong (grill Q10).
#[test]
fn an_unverifiable_runs_injected_settings_are_retained_across_a_restart() {
    let account = account("launch-file-unverifiable");
    let script = Script::new(&account, "launch-file-unverifiable")
        .holding()
        .fires(&session_start(FIRST, "startup"));
    let daemon = daemon_running(&account, &script);
    new_claude(&account);
    let before = launch_files(&account);
    assert_eq!(before.len(), 1, "{before:?}");

    // The daemon goes without watching its child end, so the Run is recorded
    // unverifiable.
    daemon.signal(rustix::process::Signal::TERM);
    let (_status, _log) = daemon.wait();
    wait_until(SETTLE, || !support::lock_is_held(&account.lock()));
    let restarted = daemon_running(&account, &script);
    // The sweep runs before the endpoint is bound, so by the time a client can
    // connect it has already decided.
    let mut client = RawClient::connect(&account.socket());
    client.establish();

    assert_eq!(
        launch_files(&account),
        before,
        "an unverifiable owner's file was destroyed",
    );

    drop(restarted);
}

/// A file whose owning Run the log has never heard of is a creation remnant of
/// a launch that was never committed, and one of the three classes this sweep
/// may remove — while a file Corral never wrote is left alone (grill Q10).
#[test]
fn a_startup_sweep_removes_a_remnant_no_launch_committed() {
    let account = account("launch-file-remnant");
    let script = Script::new(&account, "launch-file-remnant");
    let daemon = daemon_running(&account, &script);
    daemon.signal(rustix::process::Signal::TERM);
    let (_status, _log) = daemon.wait();
    wait_until(SETTLE, || !support::lock_is_held(&account.lock()));

    let launch_dir = account.state_dir().join("launch");
    let orphan = launch_dir.join("corral-launch-44444444-4444-4444-8444-444444444444.json");
    std::fs::write(&orphan, "{}").expect("a remnant");
    let partial =
        launch_dir.join("corral-launch-55555555-5555-4555-8555-555555555555.json.partial");
    std::fs::write(&partial, "half").expect("a partial write");
    let stranger = launch_dir.join("someone-elses.json");
    std::fs::write(&stranger, "{}").expect("a stranger's file");

    let restarted = daemon_running(&account, &script);
    let mut client = RawClient::connect(&account.socket());
    client.establish();

    assert!(!orphan.exists(), "a remnant survived the sweep");
    assert!(!partial.exists(), "a partial write survived the sweep");
    assert!(
        stranger.exists(),
        "the sweep touched a file it did not write"
    );

    drop(restarted);
}

/// The CLI's two namespaces, kept apart. An unknown first word is refused by
/// name and told the raw-command form, never guessed at (grill Q6).
#[test]
fn the_command_line_keeps_the_provider_and_command_namespaces_apart() {
    let account = account("cli-namespaces");
    let script = Script::new(&account, "cli-namespaces");
    let daemon = daemon_running(&account, &script);

    let refused = support::run(
        account
            .corral()
            .arg("new")
            .arg("bash")
            .stdin(std::process::Stdio::null()),
    );
    let stderr = support::stderr(&refused);
    assert!(!refused.status.success(), "{stderr}");
    assert!(
        stderr.contains("Corral does not know how to start bash"),
        "{stderr}",
    );
    assert!(
        stderr.contains("For a plain command, use: corral new -- bash"),
        "{stderr}",
    );

    assert!(script.launches().is_empty(), "{:?}", script.launches());

    // The raw form still means exactly what it always meant.
    let raw = support::run(
        account
            .corral()
            .arg("new")
            .arg("--")
            .arg("/usr/bin/true")
            .stdin(std::process::Stdio::null()),
    );
    assert!(
        support::stderr(&raw).contains("session "),
        "{}",
        support::stderr(&raw)
    );

    drop(daemon);
}

/// The verb a person types, and the verb a person reads.
///
/// `corral continue` is the product surface of `session.resume`: two
/// vocabularies on purpose, and this is where they meet (`PRODUCT.md` §8).
#[test]
fn the_command_line_continues_a_session_by_the_start_of_its_id() {
    let account = account("cli-continue");
    let script = Script::new(&account, "cli-continue").fires(&session_start(FIRST, "startup"));
    let daemon = daemon_running(&account, &script);
    let session = new_claude(&account);

    let mut client = RawClient::connect(&account.socket());
    client.establish();
    wait_until(provider::DELIVERED, || {
        listed(&sessions(&mut client, 1), &session).is_some_and(|listed| {
            external_id(listed) == Some(FIRST) && listed["execution_state"] == "exited"
        })
    });

    // A prefix is a convenience, never an identity.
    let continued = support::run(
        account
            .corral()
            .arg("continue")
            .arg(&session[..8])
            .stdin(std::process::Stdio::null()),
    );
    let stderr = support::stderr(&continued);
    assert!(continued.status.success(), "{stderr}");
    assert!(stderr.contains(&format!("session {session}")), "{stderr}");

    wait_until(SETTLE, || script.launches().len() == 2);
    assert!(
        script.launches()[1].starts_with(&format!("--resume {FIRST} ")),
        "{:?}",
        script.launches(),
    );

    drop(daemon);
}

/// The injection is Corral's to make, and a caller's own `--settings` would
/// take its place: verified first-party, the last one wins (matrix scenario
/// 8). Refused rather than silently dropped or silently honoured — the first
/// would decide a person's configuration for them, the second would leave a
/// session Corral believes it is watching and is not.
#[test]
fn a_caller_cannot_pass_the_flag_corral_injects_with() {
    let account = account("caller-settings");
    let script = Script::new(&account, "caller-settings").fires(&session_start(FIRST, "startup"));
    let daemon = daemon_running(&account, &script);

    let refused = support::run(
        account
            .corral()
            .arg("new")
            .arg("claude")
            .arg("--")
            .arg("--settings")
            .arg("/tmp/theirs.json")
            .stdin(std::process::Stdio::null()),
    );

    let stderr = support::stderr(&refused);
    assert!(!refused.status.success(), "{stderr}");
    assert!(stderr.contains("--settings"), "{stderr}");
    assert!(script.launches().is_empty(), "{:?}", script.launches());
    assert!(
        launch_files(&account).is_empty(),
        "a refused launch wrote a file"
    );

    drop(daemon);
}

/// Two continuations of one Session, arriving together under different command
/// ids. The in-flight table dedupes by command id and cannot see this; without
/// per-Session serialization both would find nothing running, both would
/// spawn, and two provider processes would be driving one conversation — which
/// the version matrix records a provider will happily allow (grill Q7).
#[test]
fn two_continuations_of_one_session_start_one_runtime() {
    let account = account("continue-race");
    let script = Script::new(&account, "continue-race").fires(&session_start(FIRST, "startup"));
    let daemon = daemon_running(&account, &script);
    let session = new_claude(&account);

    let mut client = RawClient::connect(&account.socket());
    client.establish();
    wait_until(provider::DELIVERED, || {
        listed(&sessions(&mut client, 1), &session).is_some_and(|listed| {
            external_id(listed) == Some(FIRST) && listed["execution_state"] == "exited"
        })
    });

    let socket = account.socket();
    let racers: Vec<std::thread::JoinHandle<Value>> = (0..2)
        .map(|which| {
            let socket = socket.clone();
            let session = session.clone();
            std::thread::spawn(move || {
                let mut client = RawClient::connect(&socket);
                client.establish();
                client
                    .request(
                        1,
                        "session.resume",
                        Some(json!({
                            "command_id": format!("resume-{which}"),
                            "session_id": session,
                        })),
                    )
                    .expect("session.resume answered")
            })
        })
        .collect();
    let answers: Vec<Value> = racers
        .into_iter()
        .map(|racer| racer.join().expect("a racer finished"))
        .collect();

    // Exactly one continuation happened, and one runtime was started for it.
    let accepted = answers
        .iter()
        .filter(|answer| answer["outcome"]["result"]["run_id"].is_string())
        .count();
    assert_eq!(accepted, 1, "{answers:#?}");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        script.launches().len(),
        2,
        "one launch and one continuation: {:?}",
        script.launches(),
    );
    // The loser is told to send it again rather than that its request was
    // wrong: nothing about it was.
    let refused = answers
        .iter()
        .find(|answer| answer["outcome"]["error"].is_object())
        .expect("one was refused");
    assert_eq!(error_code(refused), Some("busy"), "{refused}");

    drop(daemon);
}

/// A continuation that fails after Corral has minted its launch must undo
/// exactly what it made. The Session already existed and already had evidence
/// — its provider, its identity, the last thing it reported — and blanking
/// that would leave a Claude session rendering as though Corral never knew.
#[test]
fn a_failed_continuation_leaves_the_sessions_provider_facts_standing() {
    let account = account("continue-undo");
    let script = Script::new(&account, "continue-undo")
        .fires(&session_start(FIRST, "startup"))
        .fires(&provider::stop(FIRST));
    let daemon = daemon_running(&account, &script);
    let session = new_claude(&account);

    let mut client = RawClient::connect(&account.socket());
    client.establish();
    wait_until(provider::DELIVERED, || {
        listed(&sessions(&mut client, 1), &session).is_some_and(|listed| {
            external_id(listed) == Some(FIRST)
                && agent_event_kind(listed) == Some("turn_ended")
                && listed["execution_state"] == "exited"
        })
    });

    // The agent is no longer where the launch will look for it, so the
    // continuation fails at the spawn — after its token and settings file
    // exist.
    std::fs::remove_file(account.scratch().join("bin/claude")).expect("remove the stand-in");
    let refusal = client
        .request(
            2,
            "session.resume",
            Some(json!({"command_id": "resume-1", "session_id": session})),
        )
        .expect("session.resume answered");
    assert!(refusal["outcome"]["error"].is_object(), "{refusal}");

    let all = sessions(&mut client, 3);
    let row = listed(&all, &session).expect("the session is still listed");
    assert_eq!(provider_name(row), Some("claude"), "{row}");
    assert_eq!(external_id(row), Some(FIRST), "{row}");
    assert_eq!(agent_event_kind(row), Some("turn_ended"), "{row}");
    // And what the failed launch did make is gone.
    assert!(
        launch_files(&account).is_empty(),
        "{:?}",
        launch_files(&account)
    );

    drop(daemon);
}

/// The refusal a client was given, as a person would read it.
fn refused_with(frame: &Value) -> String {
    frame["outcome"]["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

/// Deliver one hook message by hand, the way a relay would.
///
/// Used where the scenario is about a message the relay would never compose —
/// a forgotten token, a version nobody speaks — which is exactly what the
/// endpoint has to answer for.
fn deliver_by_hand(account: &TestAccount, delivery: &Value) {
    use std::io::Write as _;
    let socket = provider::hook_socket(account);
    let mut stream =
        std::os::unix::net::UnixStream::connect(&socket).expect("the hook endpoint is listening");
    let frame = json!({
        "type": "request",
        "id": 0,
        "method": "hook.deliver",
        "params": delivery,
    });
    let mut line = serde_json::to_vec(&frame).expect("encodable");
    line.push(b'\n');
    stream.write_all(&line).expect("write a delivery");
    stream.flush().expect("flush a delivery");
}
