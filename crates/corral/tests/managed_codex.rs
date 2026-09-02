//! End-to-end: a managed Codex session, on the same seam a managed Claude
//! session runs on.
//!
//! The path is the real one and the only substitution is the agent: the daemon
//! composes the `-c notify=[…]` override, spawns through the runtime, and the
//! stand-in reads that override off its own command line and runs it — which
//! is the real `corral hook-relay` in its argv-payload mode, delivering over
//! the real hook endpoint (`AGENTS.md` §Tests).
//!
//! What these prove that `managed_provider` cannot is that the seam holds two
//! implementations: a provider with one event family, no session start, no
//! injected file, and identity that arrives at the first completed turn rather
//! than at startup (ADR 0009).

// The repository allows unwrap/expect in tests; that setting does not reach
// helpers that sit outside a `#[test]` function in an integration-test crate.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod support;

use std::time::Duration;

use serde_json::json;
use support::provider::{
    self, Script, agent_event_kind, external_id, launch_files, listed, provider_name,
    recorded_kinds, sessions, turn_complete,
};
use support::wire::{RawClient, error_code, refused_with};
use support::{DaemonProcess, SETTLE, TestAccount, wait_until};

/// Thread ids in the shape Codex mints them: UUIDv7.
const FIRST: &str = "01a0576f-0ecc-7b21-9719-f38f9e4ef933";
const SECOND: &str = "01a05771-1d2e-7c40-8a51-9b0e6b2f4c17";

/// Long enough that a daemon holding no live session does not idle out while a
/// test is still asking it questions.
const GRACE: Duration = Duration::from_secs(30);

/// An account whose `codex` is the scripted stand-in.
fn account(name: &str) -> TestAccount {
    TestAccount::new(name)
        .with_mock_provider("codex")
        .with_idle_grace(GRACE)
}

fn daemon_running(account: &TestAccount, script: &Script) -> DaemonProcess {
    account.start_daemon_with(&script.environment())
}

/// `corral new codex`, and the Session id it printed.
fn new_codex(account: &TestAccount) -> String {
    let output = support::run(
        account
            .corral()
            .arg("new")
            .arg("codex")
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

/// The whole of what a managed Codex session earns from one completed turn:
/// its identity, and a past-tense fact rendered by the projection that already
/// existed.
#[test]
fn a_managed_codex_launch_learns_its_identity_from_a_completed_turn() {
    let account = account("codex-attested");
    let script = Script::new(&account, "codex-attested")
        .holding()
        .fires(&turn_complete(FIRST));
    let daemon = daemon_running(&account, &script);

    let session = new_codex(&account);

    let mut client = RawClient::connect(&account.socket());
    client.establish();
    // The identity travels the real path — stand-in, relay in argv-payload
    // mode, endpoint, ingestion, store — so the wait is for delivery rather
    // than for a clock.
    wait_until(provider::DELIVERED, || {
        listed(&sessions(&mut client, 1), &session).and_then(external_id) == Some(FIRST)
    });

    let all = sessions(&mut client, 2);
    let row = listed(&all, &session).expect("the session that reported");
    assert_eq!(provider_name(row), Some("codex"));
    // The one fact Codex can honestly report. No start, no awaiting-input, no
    // end is synthesized to make the surface look like Claude's (ADR 0009 D3).
    assert_eq!(agent_event_kind(row), Some("turn_ended"));
    assert_eq!(row["execution_state"], "running");

    // The launch carried the override, first, ahead of anything a caller could
    // pass — and the stand-in found a runnable program inside it, which is the
    // only reason the identity above arrived at all.
    let launches = script.launches();
    assert_eq!(launches.len(), 1, "{launches:?}");
    assert!(launches[0].starts_with("-c notify=["), "{launches:?}");
    assert!(launches[0].contains("--payload-argv"), "{launches:?}");

    // And it wrote nothing: this provider's whole injection rides its argv, so
    // the launch-file lifecycle has nothing to own (ADR 0009 D1).
    assert!(
        launch_files(&account).is_empty(),
        "{:?}",
        launch_files(&account)
    );

    drop(daemon);
}

/// A Corral-managed runtime may exist without ever acquiring a provider
/// session identity, and Corral says exactly that rather than guessing at what
/// Codex left behind (ADR 0009 D3, grill Q3).
#[test]
fn a_session_that_completes_no_turn_never_binds_and_says_so() {
    let account = account("codex-zero-turn");
    let script = Script::new(&account, "codex-zero-turn");
    let daemon = daemon_running(&account, &script);
    let session = new_codex(&account);

    let mut client = RawClient::connect(&account.socket());
    client.establish();
    wait_until(provider::DELIVERED, || {
        listed(&sessions(&mut client, 1), &session)
            .is_some_and(|listed| listed["execution_state"] == "exited")
    });

    let all = sessions(&mut client, 2);
    let row = listed(&all, &session).expect("the session that reported nothing");
    assert_eq!(external_id(row), None);
    assert_eq!(
        provider_name(row),
        Some("codex"),
        "the product is known from the launch, not from a hook",
    );
    assert_eq!(agent_event_kind(row), None);
    // One binding, and it is the managed runtime's — the edge Corral created
    // and knows by construction. No provider-session binding was added,
    // because nothing ever reported an identity to bind.
    let kinds = recorded_kinds(&account.registry());
    assert_eq!(
        kinds.iter().filter(|kind| *kind == "binding-added").count(),
        1,
        "{kinds:?}",
    );

    let refusal = client
        .request(
            3,
            "session.resume",
            Some(json!({"command_id": "resume-1", "session_id": session})),
        )
        .expect("session.resume answered");
    assert_eq!(
        error_code(&refusal),
        Some("session_not_continuable"),
        "{refusal}"
    );
    assert!(
        refused_with(&refusal).contains("has not learned which provider session this is"),
        "{}",
        refused_with(&refusal),
    );

    drop(daemon);
}

/// A fresh managed session may not be a native resume in disguise.
///
/// `codex resume <id>` attaches to a conversation that may already have a
/// Corral-managed process on it, and `session.resume` is the path that holds a
/// per-Session continuation claim precisely so two processes cannot drive one
/// conversation. Reaching that through `session.new` would walk around the
/// claim, and binding uniqueness cannot repair it: that check runs when the
/// second process reports a completed turn, which is after both have been
/// writing (ADR 0009 D1, grill Q7).
///
/// Refused before anything is minted, written, or spawned.
#[test]
fn a_fresh_session_may_not_be_a_native_resume() {
    let account = account("codex-no-native-resume");
    let script = Script::new(&account, "codex-no-native-resume");
    let daemon = daemon_running(&account, &script);

    let mut client = RawClient::connect(&account.socket());
    client.establish();
    let refusal = client
        .request(
            1,
            "session.new",
            Some(json!({
                "command_id": "new-1",
                "provider": "codex",
                "args": ["resume", FIRST],
                "cwd": provider::workdir(&account).to_string_lossy(),
            })),
        )
        .expect("session.new answered");

    assert_eq!(error_code(&refusal), Some("invalid_params"), "{refusal}");
    assert!(
        refused_with(&refusal).contains("resume"),
        "the refusal names what the person wrote: {}",
        refused_with(&refusal),
    );
    // Nothing ran, and no Session exists to have run it.
    assert!(script.launches().is_empty(), "{:?}", script.launches());
    assert!(
        sessions(&mut client, 2).is_empty(),
        "{:?}",
        sessions(&mut client, 2)
    );

    drop(daemon);
}

/// Codex's way of starting over inside one runtime is a second thread, and a
/// second thread over one launch token is the existing contested path — now
/// exercised on a second provider, with nothing added to its semantics
/// (ADR 0004 D8, ADR 0009 D3).
#[test]
fn a_second_thread_in_one_runtime_contests_the_identity() {
    let account = account("codex-contested");
    let script = Script::new(&account, "codex-contested")
        .holding()
        .fires(&turn_complete(FIRST))
        .fires(&turn_complete(SECOND));
    let daemon = daemon_running(&account, &script);
    let session = new_codex(&account);

    let mut client = RawClient::connect(&account.socket());
    client.establish();
    // The contest is observable as the withdrawal of the current claim — but
    // only once the log holds it. Both turns render the same live fact, and
    // the first turn's fact is published before its identity is established,
    // so "no id yet, turn ended" is also what the first turn looks like for a
    // moment; a wait that stopped there would read the first identity landing
    // instead of the second withdrawing it. The withdrawal follows the durable
    // contest, so both together mean a settled session.
    wait_until(provider::DELIVERED, || {
        recorded_kinds(&account.registry())
            .iter()
            .any(|kind| kind == "binding-contested")
            && listed(&sessions(&mut client, 1), &session)
                .is_some_and(|listed| external_id(listed).is_none())
    });

    let all = sessions(&mut client, 2);
    let row = listed(&all, &session).expect("the contested session");
    assert_eq!(external_id(row), None);
    assert_eq!(
        provider_name(row),
        Some("codex"),
        "the product is still known"
    );
    assert_eq!(
        agent_event_kind(row),
        Some("turn_ended"),
        "facts belonging to the managed runtime keep arriving",
    );
    let kinds = recorded_kinds(&account.registry());
    assert_eq!(
        kinds
            .iter()
            .filter(|kind| *kind == "binding-contested")
            .count(),
        1,
        "{kinds:?}",
    );

    let refusal = client
        .request(
            3,
            "session.resume",
            Some(json!({"command_id": "resume-1", "session_id": session})),
        )
        .expect("session.resume answered");
    assert_eq!(
        error_code(&refusal),
        Some("session_not_continuable"),
        "{refusal}"
    );
    assert!(
        refused_with(&refusal).contains("contradicts the one Corral accepted"),
        "{}",
        refused_with(&refusal),
    );

    drop(daemon);
}

/// The same Session, a new Run, and the verb the provider itself prints on
/// exit — with the override still in front of the provider string it carries.
///
/// The confirmation is the other half. Codex reports no session start, so the
/// moment a durable `BindingConfirmed` records is the first time a Run observes
/// the identity again — once per Run, not once per turn (ADR 0009 D3).
#[test]
fn continuing_a_codex_session_composes_the_resume_verb_and_confirms_once() {
    let account = account("codex-continue");
    let script = Script::new(&account, "codex-continue")
        .fires(&turn_complete(FIRST))
        .fires(&turn_complete(FIRST));
    let daemon = daemon_running(&account, &script);
    let session = new_codex(&account);

    let mut client = RawClient::connect(&account.socket());
    client.establish();
    wait_until(provider::DELIVERED, || {
        listed(&sessions(&mut client, 1), &session).is_some_and(|listed| {
            external_id(listed) == Some(FIRST) && listed["execution_state"] == "exited"
        })
    });
    // Two completed turns in one Run establish the identity once and confirm
    // nothing: a confirmation per turn would grow the log by a fact for every
    // prompt without recording anything the last one did not. Two bindings —
    // the managed runtime's, and the provider session's.
    let kinds = recorded_kinds(&account.registry());
    assert_eq!(
        kinds.iter().filter(|kind| *kind == "binding-added").count(),
        2,
        "{kinds:?}",
    );
    assert!(
        !kinds.iter().any(|kind| kind == "binding-confirmed"),
        "{kinds:?}",
    );

    let answer = client
        .request(
            2,
            "session.resume",
            Some(json!({"command_id": "resume-1", "session_id": session})),
        )
        .expect("session.resume answered");
    assert_eq!(
        answer["outcome"]["result"]["session_id"],
        session.as_str(),
        "{answer}"
    );

    wait_until(SETTLE, || script.launches().len() == 2);
    let launches = script.launches();
    assert!(
        launches[1].starts_with("-c notify=["),
        "the override sits where no provider string can reach it: {launches:?}",
    );
    assert!(
        launches[1].ends_with(&format!("resume {FIRST}")),
        "{launches:?}",
    );
    assert_ne!(
        launches[0], launches[1],
        "a continuation carries a token of its own"
    );

    // The continued Run reports the same thread, and that re-observation is
    // written down exactly once however many turns it completes.
    wait_until(provider::DELIVERED, || {
        recorded_kinds(&account.registry())
            .iter()
            .any(|kind| kind == "binding-confirmed")
    });
    let kinds = recorded_kinds(&account.registry());
    assert_eq!(
        kinds
            .iter()
            .filter(|kind| *kind == "binding-confirmed")
            .count(),
        1,
        "{kinds:?}",
    );
    assert_eq!(
        kinds.iter().filter(|kind| *kind == "binding-added").count(),
        2,
        "a continuation reuses both the runtime binding and the identity it \
         earned: {kinds:?}",
    );

    drop(daemon);
}
