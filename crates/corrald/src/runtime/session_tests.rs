use std::ffi::OsString;
use std::time::Duration;

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
struct Running(Option<SessionHandle>);

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(handle) = self.0.as_ref() {
            handle.shut_down();
        }
    }
}

impl Running {
    /// Hand the handle to something that owns it from here on.
    fn into_handle(mut self) -> SessionHandle {
        self.0.take().expect("a running session")
    }
}

impl std::ops::Deref for Running {
    type Target = SessionHandle;

    fn deref(&self) -> &SessionHandle {
        self.0.as_ref().expect("a running session")
    }
}

fn started(script: &str) -> Running {
    Running(Some(
        start(
            &request("/bin/sh", &["-c", script]),
            GEOMETRY,
            CorralSessionId::mint(),
            RunId::mint(),
        )
        .expect("the session starts"),
    ))
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
    let handle = start(
        &request("/bin/sh", &["-c", "--token=sk-secret sleep 30"]),
        GEOMETRY,
        CorralSessionId::mint(),
        RunId::mint(),
    )
    .expect("the session starts");

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
    let (screen, reaper) = runtime.split();
    let reader = screen.reader().expect("clone the reader");
    // What a lost screen thread looks like from here: the end this reader
    // sends to is already gone.
    let (asks, questions) = sync_channel(1);
    drop(questions);

    let (done, waited) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        read_pty(reader, reaper, group, asks);
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
}
