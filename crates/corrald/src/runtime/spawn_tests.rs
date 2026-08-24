//! The PTY spawn compatibility suite.
//!
//! This suite is the vendored backend patch's regression
//! (`third_party/portable-pty/CORRAL_PATCHES.md`) and the removal condition
//! for the vendor: an upstream release with an equivalent fix has to pass all
//! of it, on macOS and Linux, before Corral goes back to the published crate.
//!
//! The load-bearing pair is `spawn_bad_shebang_is_error` against
//! `spawn_exit_1_is_distinguishable_from_exec_failure`. Unpatched, both
//! produced the same observation — a child that exits 1 — which would have let
//! a command that never ran be recorded as a Run that started and exited.

use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use super::*;
use crate::runtime::launch::{LaunchRejection, LaunchRequest};

static COUNTER: AtomicU32 = AtomicU32::new(0);

const GEOMETRY: PtyGeometry = PtyGeometry::expect_valid(31, 113);

/// A scratch file removed however the test ends.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn scratch(name: &str, contents: &[u8], mode: u32) -> Scratch {
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "corral-spawn-{}-{unique}-{name}",
        std::process::id()
    ));
    std::fs::write(&path, contents).expect("write the scratch file");
    let mut permissions = std::fs::metadata(&path)
        .expect("read the scratch file's metadata")
        .permissions();
    permissions.set_mode(mode);
    std::fs::set_permissions(&path, permissions).expect("set the scratch file's mode");
    Scratch(path)
}

fn request(program: &str, args: &[&str]) -> LaunchRequest {
    LaunchRequest::new(
        program,
        args.iter().map(std::ffi::OsString::from),
        std::env::temp_dir(),
    )
    .expect("a valid launch request")
}

/// A spawned runtime, split the way production splits it, and torn down
/// however the test ends.
///
/// Without the teardown every case here leaves a live child behind for the
/// length of its own sleep: setup with no matching end.
struct Started {
    screen: ManagedTerminal,
    reaper: ChildReaper,
    group: Option<ChildGroup>,
}

impl std::fmt::Debug for Started {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Started")
            .field("group", &self.group)
            .finish()
    }
}

impl Drop for Started {
    fn drop(&mut self) {
        if let Some(group) = self.group {
            group.hang_up();
        }
        let _ = self.reaper.wait();
    }
}

fn started(request: &LaunchRequest) -> Result<Started, SpawnError> {
    let runtime = spawn(request, GEOMETRY)?;
    let group = runtime.child_group();
    let (screen, reaper) = runtime.split();
    Ok(Started {
        screen,
        reaper,
        group,
    })
}

/// Read the terminal to EOF so the child never blocks on a full buffer, then
/// return what it wrote.
fn drain(runtime: &Started) -> std::sync::mpsc::Receiver<Vec<u8>> {
    let mut reader = runtime.screen.reader().expect("clone the reader");
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut output = Vec::new();
        let _ = reader.read_to_end(&mut output);
        let _ = sender.send(output);
    });
    receiver
}

#[test]
fn spawn_missing_executable_is_error() {
    let error =
        started(&request("/definitely/not/here", &[])).expect_err("a missing program cannot start");

    assert!(matches!(error, SpawnError::Exec(_)), "{error}");
}

#[test]
fn spawn_non_executable_is_error() {
    let file = scratch("no-exec-bit", b"#!/bin/sh\necho hi\n", 0o644);

    let error = started(&request(&file.0.to_string_lossy(), &[]))
        .expect_err("a file without an exec bit cannot start");

    assert!(matches!(error, SpawnError::Exec(_)), "{error}");
}

/// The case the vendor patch exists for: the file passes every pre-fork check
/// and `execve` fails afterwards, because its interpreter does not exist.
#[test]
fn spawn_bad_shebang_is_error() {
    let file = scratch("bad-shebang", b"#!/definitely/not/here\n", 0o755);

    let error = started(&request(&file.0.to_string_lossy(), &[]))
        .expect_err("a dangling interpreter cannot start");

    assert!(matches!(error, SpawnError::Exec(_)), "{error}");
}

/// The other half of the pair. A program that really ran and failed must stay
/// distinguishable from one that never exec'd: the first is a Run that ended,
/// the second is a Run that never existed.
#[test]
fn spawn_exit_1_is_distinguishable_from_exec_failure() {
    let mut runtime =
        started(&request("/bin/sh", &["-c", "exit 1"])).expect("a real program starts");
    let output = drain(&runtime);

    assert_eq!(
        runtime.reaper.wait().expect("reap the child"),
        ExitCause::Failed
    );
    let _ = output.recv_timeout(std::time::Duration::from_secs(5));

    let never_started = spawn(
        &request(
            &scratch("pair-bad-shebang", b"#!/definitely/not/here\n", 0o755)
                .0
                .to_string_lossy(),
            &[],
        ),
        GEOMETRY,
    );
    assert!(
        never_started.is_err(),
        "a command that never exec'd must not look like one that exited 1"
    );
}

/// Every nonzero exit is a Run that failed, whatever the number was. The
/// platform's code is mapped here and goes no further: the domain describes an
/// ending in `ExitCause`, and a number crossing that line would leave every
/// consumer deciding for itself what `42` means (ADR 0002).
#[test]
fn any_nonzero_exit_is_a_run_that_failed() {
    let mut runtime = started(&request("/bin/sh", &["-c", "exit 42"])).expect("the program starts");
    let output = drain(&runtime);

    assert_eq!(
        runtime.reaper.wait().expect("reap the child"),
        ExitCause::Failed
    );
    let _ = output.recv_timeout(std::time::Duration::from_secs(5));
}

/// A signal is its own ending, including Corral's own hang-up. That a signal
/// ended the child is a fact about the ending, not a claim about who sent it.
#[test]
fn a_signalled_child_ends_as_terminated() {
    let mut runtime =
        started(&request("/bin/sh", &["-c", "kill -TERM $$"])).expect("the program starts");
    let output = drain(&runtime);

    assert_eq!(
        runtime.reaper.wait().expect("reap the child"),
        ExitCause::Terminated
    );
    let _ = output.recv_timeout(std::time::Duration::from_secs(5));
}

/// The child must lead its own session and process group: that is what makes
/// the pty its controlling terminal, and what gives teardown a group to
/// target rather than a single pid.
#[test]
fn pty_child_is_session_and_process_group_leader() {
    let mut runtime = started(&request(
        "/bin/sh",
        &["-c", "ps -o pid,pgid -p $$ | tail -1"],
    ))
    .expect("the program starts");
    let output = drain(&runtime);
    let leader = runtime.screen.process_group_leader();
    let pid = runtime.group.map(ChildGroup::as_pid);

    let _ = runtime.reaper.wait().expect("reap the child");
    let reported = output
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("the child's report");
    let reported = String::from_utf8_lossy(&reported);
    let mut columns = reported.split_whitespace();
    let child_pid: u32 = columns.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let child_pgid: u32 = columns.next().and_then(|v| v.parse().ok()).unwrap_or(0);

    assert_eq!(Some(child_pid), pid, "the pid Corral holds is the child's");
    assert_eq!(child_pid, child_pgid, "the child leads its process group");
    assert_eq!(
        leader.map(|leader| leader as u32),
        Some(child_pgid),
        "the terminal reports that group as its foreground group"
    );
}

/// Geometry set at spawn reaches the child, and a later resize replaces it.
#[test]
fn pty_resize_round_trips() {
    let mut runtime = started(&request(
        "/bin/sh",
        &["-c", "stty size; read _ignored; stty size"],
    ))
    .expect("the program starts");
    let mut reader = runtime.screen.reader().expect("clone the reader");
    let mut writer = runtime.screen.writer().expect("take the writer");

    // Read until the line is complete: a pty returns whatever is ready, so a
    // single read can hand back "31 " and nothing more. Asserting on one read
    // is the classic flaky-test shape.
    let mut first = Vec::new();
    let mut byte = [0_u8; 1];
    while !first.contains(&b'\n') {
        let read = reader.read(&mut byte).expect("the first size line");
        if read == 0 {
            break;
        }
        first.push(byte[0]);
    }
    assert!(
        String::from_utf8_lossy(&first).contains("31 113"),
        "the child starts at the geometry it was spawned with, got {:?}",
        String::from_utf8_lossy(&first)
    );

    runtime
        .screen
        .resize(PtyGeometry::expect_valid(24, 80))
        .expect("resize the terminal");
    assert_eq!(
        runtime.screen.geometry().expect("read the geometry back"),
        PtyGeometry::expect_valid(24, 80)
    );

    use std::io::Write;
    writer.write_all(b"\n").expect("let the child continue");
    writer.flush().expect("flush");

    let mut rest = Vec::new();
    let _ = reader.read_to_end(&mut rest);
    assert!(
        String::from_utf8_lossy(&rest).contains("24 80"),
        "the child sees the new geometry, got {:?}",
        String::from_utf8_lossy(&rest)
    );

    let _ = runtime.reaper.wait();
}

/// The child is told what terminal it is talking to, and that is Corral's
/// emulator rather than whatever the daemon's own environment says.
#[test]
fn the_child_is_told_which_terminal_it_talks_to() {
    let mut runtime = started(&request("/bin/sh", &["-c", "printf '[%s]' \"$TERM\""]))
        .expect("the program starts");
    let output = drain(&runtime);

    let _ = runtime.reaper.wait().expect("reap the child");
    let reported = output
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("the child's report");

    assert!(
        String::from_utf8_lossy(&reported).contains("[xterm-256color]"),
        "got {:?}",
        String::from_utf8_lossy(&reported)
    );
}

/// A rejection made before the backend is called is a different outcome from
/// a spawn failure, and neither is a Run.
#[test]
fn a_refused_request_never_reaches_the_backend() {
    let missing = std::env::temp_dir().join("corral-no-such-directory-ever");
    let _ = std::fs::remove_dir_all(&missing);

    let rejection = LaunchRequest::new("/bin/sh", std::iter::empty(), &missing)
        .expect_err("the request is refused");

    assert!(matches!(
        rejection,
        LaunchRejection::WorkingDirectoryMissing(_)
    ));
}

/// The teardown window closes before the reaper waits, and a signal that
/// arrives afterwards does nothing. Otherwise a hang-up queued between the
/// reap and its announcement would target a pid the kernel has released
/// (ADR 0007 L4).
#[test]
fn a_closed_teardown_window_signals_nothing() {
    let mut runtime = started(&request("/bin/sh", &["-c", "sleep 30"])).expect("the child starts");
    let group = runtime.group.expect("a pid");
    let window = TeardownWindow::open(Some(group));

    window.close();
    window.hang_up();

    // Still ours to end: the window refused, so nothing was signalled.
    assert_eq!(
        runtime.screen.process_group_leader().map(|it| it as u32),
        Some(group.as_pid()),
        "the child was hung up through a window that had closed"
    );

    // And an open window does signal, so the assertion above is not vacuous.
    let open = TeardownWindow::open(Some(group));
    open.hang_up();
    let _ = runtime.reaper.wait();
}
