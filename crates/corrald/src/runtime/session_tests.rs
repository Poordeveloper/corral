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

fn started(script: &str) -> SessionHandle {
    start(
        &request("/bin/sh", &["-c", script]),
        GEOMETRY,
        CorralSessionId::mint(),
        RunId::mint(),
    )
    .expect("the session starts")
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
    sessions.insert(started("sleep 30"));
    sessions.insert(started("sleep 30"));

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

    sessions.insert(handle);
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
    let mut handle = started("sleep 30");
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
