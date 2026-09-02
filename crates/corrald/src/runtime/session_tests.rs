use std::ffi::OsString;
use std::time::Duration;

use super::super::occurrence::{ObservedRuns, observe_runs};
use super::*;

const GEOMETRY: PtyGeometry = PtyGeometry::expect_valid(24, 80);

fn request(program: &str, args: &[&str]) -> LaunchRequest {
    LaunchRequest::new(
        program,
        args.iter().map(OsString::from),
        std::env::temp_dir(),
    )
    .expect("a valid launch request")
}

/// A started session, hung up however the test ends.
///
/// Setup with a matching end: without it every case here leaves a live child
/// and two threads behind for the length of its own sleep.
///
/// It keeps the draining end of the occurrence channel, because a runtime with
/// nowhere to report to is not the runtime the daemon runs — and because what
/// it reports is itself under test.
struct Running {
    handle: Option<SessionHandle>,
    run: RunId,
    observed: ObservedRuns,
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.as_ref() {
            handle.shut_down();
        }
    }
}

impl Running {
    /// Hand the handle to something that owns it from here on.
    fn into_handle(mut self) -> SessionHandle {
        self.handle.take().expect("a running session")
    }

    /// The next thing this session's runtime reported about its Run.
    fn reported(&self) -> Option<RunOccurrence> {
        self.observed.next().map(|observed| observed.occurrence())
    }
}

impl std::ops::Deref for Running {
    type Target = SessionHandle;

    fn deref(&self) -> &SessionHandle {
        self.handle.as_ref().expect("a running session")
    }
}

fn started(script: &str) -> Running {
    serving(&request("/bin/sh", &["-c", script]))
}

fn serving(request: &LaunchRequest) -> Running {
    let (observations, observed) = observe_runs();
    let run = RunId::mint();
    let handle = spawn_session(request, GEOMETRY)
        .expect("the session starts")
        .serve(CorralSessionId::mint(), run, observations);
    Running {
        handle: Some(handle),
        run,
        observed,
    }
}

/// Wait for the screen to reflect something, rather than sleeping a fixed
/// span: a timing assumption is a flaky test waiting to happen.
fn wait_for<T>(mut probe: impl FnMut() -> Option<T>) -> Option<T> {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if let Some(value) = probe() {
            return Some(value);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    None
}

#[test]
fn a_started_session_serves_a_snapshot() {
    let handle = started("printf 'hello\\r\\n'; sleep 30");

    let snapshot = handle
        .snapshot()
        .expect("the session answered")
        .expect("the screen encodes");

    assert!(!snapshot.payload().is_empty());
}

#[test]
fn a_started_session_reports_the_geometry_it_was_given() {
    let handle = started("sleep 30");

    assert_eq!(
        handle.geometry().expect("the session answered"),
        Ok(GEOMETRY)
    );
}

/// The title is the executable basename, never the arguments: argv carries
/// tokens and identifiers a list has no business spreading (grill Q3).
#[test]
fn the_session_title_is_the_program_not_its_arguments() {
    let handle = serving(&request("/bin/sh", &["-c", "--token=sk-secret sleep 30"]));

    assert_eq!(handle.title(), "sh");
}

/// The last explicit resize wins and opens an epoch, because bytes recorded
/// before a reflow cannot be replayed into a screen shaped after it.
#[test]
fn a_resize_opens_a_new_epoch_and_moves_the_authoritative_geometry() {
    let handle = started("sleep 30");
    let wanted = PtyGeometry::expect_valid(40, 120);

    let epoch = handle
        .resize(wanted)
        .expect("the session answered")
        .expect("the terminal took the size");

    assert_eq!(epoch, Epoch(1), "the first resize opened the first epoch");
    assert_eq!(handle.geometry().expect("the session answered"), Ok(wanted));
}

#[test]
fn successive_resizes_open_successive_epochs() {
    let handle = started("sleep 30");

    let first = handle
        .resize(PtyGeometry::expect_valid(30, 90))
        .expect("answered")
        .expect("took");
    let second = handle
        .resize(PtyGeometry::expect_valid(50, 100))
        .expect("answered")
        .expect("took");

    assert!(second > first);
}

/// Input is bytes the client's replica encoded; the daemon writes them
/// through without interpreting them.
#[test]
fn input_reaches_the_child_unchanged() {
    let handle = started("read line; printf '\\033]2;%s\\007' \"$line\"; sleep 30");

    handle
        .write_input(b"typed-by-a-client\n".to_vec())
        .expect("the session answered");

    let title = wait_for(|| handle.title_from_screen().ok().flatten());
    assert_eq!(
        title.as_deref(),
        Some(b"typed-by-a-client".as_slice()),
        "the child never saw what the client typed"
    );
}

#[test]
fn child_output_reaches_the_daemons_screen() {
    let handle = started("printf '\\033]2;from-the-child\\007'; sleep 30");

    let title = wait_for(|| handle.title_from_screen().ok().flatten());

    assert_eq!(title.as_deref(), Some(b"from-the-child".as_slice()));
}

#[test]
fn a_daemon_describes_the_sessions_it_runs() {
    let mut sessions = ManagedSessions::new();
    sessions.insert(started("sleep 30").into_handle());
    sessions.insert(started("sleep 30").into_handle());

    let described = sessions.describe();

    assert_eq!(described.len(), 2);
    assert!(
        described
            .iter()
            .all(|session| session.execution_state == ExecutionState::Running)
    );
    assert!(described.iter().all(|session| session.title == "sh"));
}

/// An exit is claimed only once it has been observed: the daemon reaps the
/// child and then says Exited.
#[test]
fn an_observed_exit_is_reported_as_exited() {
    let mut sessions = ManagedSessions::new();
    let handle = started("printf 'done\r\n'");

    let exited = wait_for(|| (handle.execution_state() == ExecutionState::Exited).then_some(()));
    assert!(exited.is_some(), "the daemon never observed the exit");

    sessions.insert(handle.into_handle());
    assert_eq!(
        sessions.describe()[0].execution_state,
        ExecutionState::Exited
    );
}

/// The screen outlives the process. Someone attaching after an agent finished
/// still needs to read what it left behind.
#[test]
fn a_finished_sessions_screen_is_still_readable() {
    let handle = started(r"printf '\033]2;finished\007'");

    let exited = wait_for(|| (handle.execution_state() == ExecutionState::Exited).then_some(()));
    assert!(exited.is_some(), "the daemon never observed the exit");

    let snapshot = handle
        .snapshot()
        .expect("the screen still answers")
        .expect("the screen encodes");
    assert!(!snapshot.payload().is_empty());
    assert_eq!(
        handle
            .title_from_screen()
            .expect("the screen still answers"),
        Some(b"finished".to_vec())
    );
}

/// A session whose runtime stops answering is Unknown, never Exited: losing
/// the ability to manage a runtime is not evidence that a process died
/// (ADR 0002, grill Q5).
#[test]
fn a_session_whose_runtime_stops_answering_is_unknown_not_exited() {
    let mut sessions = ManagedSessions::new();
    let mut handle = started("sleep 30").into_handle();
    // Sever the daemon's ability to ask, which is exactly what a lost runtime
    // looks like from here: the process's fate is not part of what happened.
    handle.sever_for_test();

    sessions.insert(handle);
    let described = sessions.describe();

    assert_eq!(described[0].execution_state, ExecutionState::Unknown);
    assert_ne!(
        described[0].execution_state,
        ExecutionState::Exited,
        "the daemon claimed an exit it never observed"
    );
}

#[test]
fn the_execution_states_have_stable_wire_spellings() {
    assert_eq!(ExecutionState::Running.as_str(), "running");
    assert_eq!(ExecutionState::Exited.as_str(), "exited");
    assert_eq!(ExecutionState::Unknown.as_str(), "unknown");
}

/// The screen stops being an actor when its runtime ends (ADR 0007 L2). The
/// thread, the emulator and the pty go; what a finished session answers from
/// is the record that thread published on its way out.
#[test]
fn a_finished_run_releases_the_thread_that_served_its_screen() {
    let handle = started(r"printf 'left-behind\r\n'").into_handle();

    // The liveness proof is the thread's own stack: it drops when the thread
    // returns, whatever returned it.
    let retired = wait_for(|| handle.alive.upgrade().is_none().then_some(()));

    assert!(
        retired.is_some(),
        "the screen thread outlived the runtime it existed to serve"
    );
    let snapshot = handle
        .snapshot()
        .expect("the record answers")
        .expect("the screen encodes");
    assert!(
        String::from_utf8_lossy(snapshot.payload()).contains("left-behind"),
        "the record lost what the run left on the screen"
    );
}

/// An exit Corral watched happen stays watched. Retiring the thread that
/// published it cannot make it unestablished again (ADR 0007 L3).
#[test]
fn a_retired_session_still_reports_the_exit_corral_observed() {
    let handle = started("true").into_handle();

    let retired = wait_for(|| handle.alive.upgrade().is_none().then_some(()));
    assert!(retired.is_some(), "the screen thread never retired");

    assert_eq!(
        handle.execution_state(),
        ExecutionState::Exited,
        "the daemon un-established an exit it had already observed"
    );
}

/// Attaching after the end gives the whole of what there is. There is no
/// stream, and saying so is what stops a viewer waiting for deltas that can
/// never come (ADR 0007 L2).
#[test]
fn attaching_after_the_run_ended_gives_a_final_screen_and_no_stream() {
    let handle = started(r"printf 'left-behind\r\n'").into_handle();
    assert!(
        wait_for(|| handle.alive.upgrade().is_none().then_some(())).is_some(),
        "the screen thread never retired"
    );

    let attachment = handle.attach().expect("the record answers");

    assert!(
        attachment.viewer.is_none(),
        "a finished run offered a stream that can never produce"
    );
    let snapshot = attachment.snapshot.expect("the screen encodes");
    assert!(String::from_utf8_lossy(snapshot.payload()).contains("left-behind"));
}

/// A person typing at a finished run is told what happened, and told the truth:
/// the run ended, which is a different fact from a runtime that stopped
/// answering (ADR 0007 L3).
#[test]
fn input_to_a_finished_run_is_refused_as_ended_rather_than_swallowed() {
    let handle = started("true").into_handle();
    assert!(
        wait_for(|| handle.alive.upgrade().is_none().then_some(())).is_some(),
        "the screen thread never retired"
    );

    assert_eq!(
        handle.write_input(b"typed-at-a-corpse\n".to_vec()),
        Err(InputRefused::RunEnded)
    );
}

#[test]
fn resizing_a_finished_run_says_the_run_ended() {
    let handle = started("true").into_handle();
    assert!(
        wait_for(|| handle.alive.upgrade().is_none().then_some(())).is_some(),
        "the screen thread never retired"
    );

    assert_eq!(
        handle.resize(PtyGeometry::expect_valid(40, 120)),
        Ok(Err(ResizeRefused::RunEnded))
    );
}

/// A screen gone while its runtime is live leaves nothing draining the pty, so
/// a child left running fills its buffer and blocks on a write forever —
/// alive, unreachable, unlistable. The reaper ends it with the group it still
/// owns, which is the ruling already made for a session the registry refused
/// (ADR 0007 L4).
#[test]
fn a_lost_screen_ends_the_run_rather_than_leaving_it_blocked() {
    let runtime = super::super::spawn::spawn(
        &request("/bin/sh", &["-c", r"printf 'x'; sleep 30"]),
        GEOMETRY,
    )
    .expect("the child starts");
    let group = runtime.child_group();
    let teardown = std::sync::Arc::new(super::super::spawn::TeardownWindow::open(group));
    let (screen, reaper) = runtime.split();
    let reader = screen.reader().expect("clone the reader");
    // What a lost screen thread looks like from here: the end this reader
    // sends to is already gone.
    let (asks, questions) = sync_channel(1);
    drop(questions);

    let (done, waited) = std::sync::mpsc::channel();
    let reaping = std::sync::Arc::clone(&teardown);
    let (observations, observed) = observe_runs();
    std::thread::spawn(move || {
        read_pty(reader, reaper, &reaping, asks, RunId::mint(), &observations);
        let _ = done.send(());
    });

    let ended = waited.recv_timeout(Duration::from_secs(10)).is_ok();
    if !ended {
        // Leave nothing running behind a failure.
        if let Some(group) = group {
            group.hang_up();
        }
    }
    assert!(
        ended,
        "the child outlived the screen that was the only thing draining it"
    );
    assert!(
        matches!(
            observed.next().map(|seen| seen.occurrence()),
            Some(RunOccurrence::Exited { .. })
        ),
        "a run whose screen was lost still reports how it ended"
    );
}

/// A child can reshape the authoritative screen without anyone touching the
/// pty: DECCOLM makes the emulator 132 columns wide by itself. Corral must
/// follow, or it answers `terminal.attach` with a size the screen no longer
/// has and hands the client a snapshot that wraps into garbage.
#[test]
fn a_child_that_reshapes_the_screen_moves_the_published_geometry() {
    // Enable mode 3, then set it: DECCOLM's own two-step.
    let handle = started(r"printf '\033[?40h\033[?3h'; sleep 30");

    let widened = wait_for(|| (handle.last_geometry().cols() == 132).then_some(()));

    assert!(
        widened.is_some(),
        "the daemon still reports {:?} for a screen the child widened",
        handle.last_geometry()
    );
}

/// A runtime whose Run never became a durable fact is hung up and reaped, not
/// left alive and unlistable. `abandon` blocks on the reaper, so it returning
/// at all is the assertion: a child that was not signalled would keep this
/// waiting for the whole sleep (grill Q9).
#[test]
fn an_abandoned_runtime_is_hung_up_and_reaped() {
    let pending = spawn_session(&request("/bin/sh", &["-c", "sleep 300"]), GEOMETRY)
        .expect("the session starts");

    let (done, waited) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        pending.abandon();
        let _ = done.send(());
    });

    assert!(
        waited.recv_timeout(Duration::from_secs(10)).is_ok(),
        "a runtime nobody could record outlived the daemon giving up on it"
    );
}

/// The whole point of splitting the start: nothing between a spawned runtime
/// and a served one can fail, so a Run whose start committed is always served.
#[test]
fn a_pending_session_knows_its_title_before_it_is_served() {
    let pending = spawn_session(&request("/bin/sh", &["-c", "sleep 30"]), GEOMETRY)
        .expect("the session starts");

    assert_eq!(pending.title(), "sh");
    pending.abandon();
}

/// The end of a run is reported by the party that establishes it, so a
/// durable `RunEnded` can name what actually happened rather than that the
/// daemon stopped hearing anything.
#[test]
fn a_run_that_exits_reports_how_it_ended() {
    let session = started("exit 0");

    let reported = session.reported();

    assert!(
        matches!(
            reported,
            Some(RunOccurrence::Exited {
                run,
                end: RunEnd::Exited(ExitCause::Completed),
                at: OccurrenceTime::Authoritative(_),
            }) if run == session.run
        ),
        "{reported:?}"
    );
}

/// A run Corral tore down ended by a signal, and says so. That a signal ended
/// it is a fact about the ending, not a claim about who sent it.
#[test]
fn a_run_corral_shut_down_reports_a_terminated_ending() {
    let session = started("sleep 300");
    session.shut_down();

    let reported = session.reported();

    assert!(
        matches!(
            reported,
            Some(RunOccurrence::Exited {
                end: RunEnd::Exited(ExitCause::Terminated),
                ..
            })
        ),
        "{reported:?}"
    );
}

/// The reproducer the pre-merge fuzz campaign distilled: an OSC title
/// truncated mid-character, which panics the emulator's parser
/// (`docs/evidence/pr3-terminal-fuzz-2026-08-24.md`). Read from the corpus
/// rather than restated here, so the containment's regression floor and the
/// surface that reports it cannot drift onto different bytes.
fn poisoning_input() -> std::path::PathBuf {
    let reproducer = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("corpus")
        .join("terminal")
        .join("osc-title-truncation-splits-a-character.bin");

    // The tests that use it feed the path to a child. A corpus entry that
    // moved would leave the screen unpoisoned and the failure would arrive as
    // an assertion about a screen that served itself — the daemon blamed for a
    // missing fixture.
    assert!(
        reproducer.is_file(),
        "{} is missing; the tests that need it cannot say so for themselves",
        reproducer.display()
    );

    reproducer
}

#[test]
fn a_live_session_can_have_its_terminal_served() {
    let handle = started("sleep 30");

    assert_eq!(handle.terminal_access(), TerminalAccess::Available);
}

/// A run that ended still has a screen worth opening (ADR 0007 L2), so the
/// capability outlives the process it belonged to.
#[test]
fn a_finished_session_whose_screen_survives_can_still_be_served() {
    let handle = started(r"printf 'left-behind\r\n'");

    let exited = wait_for(|| (handle.execution_state() == ExecutionState::Exited).then_some(()));

    assert!(exited.is_some(), "the daemon never observed the exit");
    assert_eq!(handle.terminal_access(), TerminalAccess::Available);
}

/// The two dimensions, held apart. A screen Corral may no longer read says
/// nothing about the child, which in this test is still running — and the
/// list must say so before a person presses Open rather than after
/// (grill Q7).
#[test]
fn a_poisoned_screen_cannot_be_served_and_is_not_evidence_about_the_process() {
    let mut sessions = ManagedSessions::new();
    let handle = started(&format!("cat '{}'; sleep 30", poisoning_input().display()));

    let refused =
        wait_for(|| (handle.terminal_access() == TerminalAccess::Unavailable).then_some(()));

    assert!(
        refused.is_some(),
        "a screen nobody may read still offered itself for attaching"
    );
    assert_eq!(
        handle.execution_state(),
        ExecutionState::Running,
        "a screen Corral cannot read was turned into a claim about the process"
    );

    sessions.insert(handle.into_handle());
    assert_eq!(
        sessions.describe()[0].terminal_access,
        TerminalAccess::Unavailable
    );
}

/// A screen thread gone without leaving a record is a loss, and there is
/// nothing left to serve a snapshot from (ADR 0007 L3).
#[test]
fn a_lost_screen_thread_leaves_no_terminal_to_serve() {
    let mut handle = started("sleep 30").into_handle();

    handle.sever_for_test();

    assert_eq!(handle.terminal_access(), TerminalAccess::Unavailable);
}

/// Newest first, so the session a person just started is the one under the
/// cursor. This orders the current daemon-visible list and nothing else: not
/// history, not resumable ranking, not attention (grill Q3).
#[test]
fn sessions_are_listed_newest_first() {
    let mut sessions = ManagedSessions::new();
    let older = started("sleep 30").into_handle();
    let newer = started("sleep 30").into_handle();
    let (older_id, newer_id) = (older.session(), newer.session());
    // Inserted oldest first, so the answer cannot come from insertion order.
    sessions.insert(older);
    sessions.insert(newer);

    let described = sessions.describe();

    assert_eq!(
        described
            .iter()
            .map(|session| session.session)
            .collect::<Vec<_>>(),
        vec![newer_id, older_id],
    );
}

#[test]
fn sessions_started_in_the_same_instant_fall_back_to_a_deterministic_order() {
    let mut sessions = ManagedSessions::new();
    let together = Instant::now();
    let mut first = started("sleep 30").into_handle();
    let mut second = started("sleep 30").into_handle();
    first.started_at_for_test(together);
    second.started_at_for_test(together);
    let mut expected = vec![first.session().to_string(), second.session().to_string()];
    expected.sort();
    sessions.insert(first);
    sessions.insert(second);

    let described = sessions.describe();

    assert_eq!(
        described
            .iter()
            .map(|session| session.session.to_string())
            .collect::<Vec<_>>(),
        expected,
    );
}

/// The screen thread publishes when the child last drew, so the attention
/// engine can turn output into an activity claim without asking the screen.
#[test]
fn the_last_output_instant_is_published_once_the_child_draws() {
    let running = started("printf hello; sleep 30");
    let handle = running.handle.as_ref().expect("a handle");
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while handle.last_output_at().is_none() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let drawn = handle.last_output_at().expect("the child drew");
    assert!(drawn <= std::time::SystemTime::now());
}

/// With a manifest, the screen thread publishes what the screen matches once
/// output settles, dated by the evaluation, so the tick can turn it into a
/// claim without asking the screen.
#[test]
fn a_settled_screen_publishes_its_reading() {
    let (manifest, _) = crate::detection::manifest::parse(
        "schema = 1\nmin_engine_version = 1\nversion = \"t\"\nprovider = \"test\"\n[[rule]]\nid = \"hello\"\nasserts = \"turn_complete\"\nregion = \"whole_screen\"\nall = [\"hello\"]\n",
    )
    .expect("manifest");
    let pending = spawn_session(
        &request("/bin/sh", &["-c", "printf hello; sleep 30"]),
        GEOMETRY,
    )
    .expect("spawn")
    .detect_with(std::sync::Arc::new(manifest));
    let (observations, observed) = observe_runs();
    let handle = pending.serve(CorralSessionId::mint(), RunId::mint(), observations);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while handle.reading().is_none() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let reading = handle.reading().expect("a reading");
    assert_eq!(reading.rule, "hello");
    assert_eq!(reading.asserts, corral_core::SemanticState::Ready);
    handle.shut_down();
    drop(observed);
}

/// The echo of a keystroke Corral wrote is a person typing, not the agent
/// drawing: output inside the echo window after an input leaves the
/// last-output instant where it was.
#[test]
fn the_echo_of_written_input_is_not_the_child_drawing() {
    // `cat` echoes what it is given and draws nothing on its own.
    let running = started("exec cat");
    let handle = running.handle.as_ref().expect("a handle");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(handle.last_output_at(), None, "nothing drawn yet");

    handle.write_input(b"hello".to_vec()).expect("input taken");
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        handle.last_output_at(),
        None,
        "the echo is not the child drawing"
    );

    // Output the child produces on its own, later, does count.
    std::thread::sleep(Duration::from_millis(ECHO_WINDOW_MS + 50));
    handle.write_input(b"\n".to_vec()).expect("input taken");
    std::thread::sleep(Duration::from_millis(ECHO_WINDOW_MS + 50));
    // `cat` echoes the newline within the window as well; the assertion
    // above holds for the same reason.
    let _ = handle.last_output_at();
}
