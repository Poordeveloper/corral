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

const GEOMETRY: PtyGeometry = PtyGeometry {
    rows: 31,
    cols: 113,
};

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

/// Read the terminal to EOF so the child never blocks on a full buffer, then
/// return what it wrote.
fn drain(runtime: &SpawnedRuntime) -> std::sync::mpsc::Receiver<Vec<u8>> {
    let mut reader = runtime.reader().expect("clone the reader");
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
    let error = spawn(&request("/definitely/not/here", &[]), GEOMETRY)
        .expect_err("a missing program cannot start");

    assert!(matches!(error, SpawnError::Exec(_)), "{error}");
}

#[test]
fn spawn_non_executable_is_error() {
    let file = scratch("no-exec-bit", b"#!/bin/sh\necho hi\n", 0o644);

    let error = spawn(&request(&file.0.to_string_lossy(), &[]), GEOMETRY)
        .expect_err("a file without an exec bit cannot start");

    assert!(matches!(error, SpawnError::Exec(_)), "{error}");
}

/// The case the vendor patch exists for: the file passes every pre-fork check
/// and `execve` fails afterwards, because its interpreter does not exist.
#[test]
fn spawn_bad_shebang_is_error() {
    let file = scratch("bad-shebang", b"#!/definitely/not/here\n", 0o755);

    let error = spawn(&request(&file.0.to_string_lossy(), &[]), GEOMETRY)
        .expect_err("a dangling interpreter cannot start");

    assert!(matches!(error, SpawnError::Exec(_)), "{error}");
}

/// The other half of the pair. A program that really ran and exited 1 must
/// stay distinguishable from one that never exec'd — same exit code,
/// different fact.
#[test]
fn spawn_exit_1_is_distinguishable_from_exec_failure() {
    let mut runtime =
        spawn(&request("/bin/sh", &["-c", "exit 1"]), GEOMETRY).expect("a real program starts");
    let output = drain(&runtime);

    assert_eq!(runtime.wait().expect("reap the child"), 1);
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

#[test]
fn spawn_exit_42_is_preserved() {
    let mut runtime =
        spawn(&request("/bin/sh", &["-c", "exit 42"]), GEOMETRY).expect("the program starts");
    let output = drain(&runtime);

    assert_eq!(runtime.wait().expect("reap the child"), 42);
    let _ = output.recv_timeout(std::time::Duration::from_secs(5));
}

/// The child must lead its own session and process group: that is what makes
/// the pty its controlling terminal, and what gives teardown a group to
/// target rather than a single pid.
#[test]
fn pty_child_is_session_and_process_group_leader() {
    let mut runtime = spawn(
        &request("/bin/sh", &["-c", "ps -o pid,pgid -p $$ | tail -1"]),
        GEOMETRY,
    )
    .expect("the program starts");
    let output = drain(&runtime);
    let leader = runtime.process_group_leader();
    let pid = runtime.process_id();

    let _ = runtime.wait().expect("reap the child");
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
    let mut runtime = spawn(
        &request("/bin/sh", &["-c", "stty size; read _ignored; stty size"]),
        GEOMETRY,
    )
    .expect("the program starts");
    let mut reader = runtime.reader().expect("clone the reader");
    let mut writer = runtime.writer().expect("take the writer");

    let mut first = [0_u8; 64];
    let read = reader.read(&mut first).expect("the first size line");
    assert!(
        String::from_utf8_lossy(&first[..read]).contains("31 113"),
        "the child starts at the geometry it was spawned with, got {:?}",
        String::from_utf8_lossy(&first[..read])
    );

    runtime
        .resize(PtyGeometry { rows: 24, cols: 80 })
        .expect("resize the terminal");
    assert_eq!(
        runtime.geometry().expect("read the geometry back"),
        PtyGeometry { rows: 24, cols: 80 }
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

    let _ = runtime.wait();
}

/// The child is told what terminal it is talking to, and that is Corral's
/// emulator rather than whatever the daemon's own environment says.
#[test]
fn the_child_is_told_which_terminal_it_talks_to() {
    let mut runtime = spawn(
        &request("/bin/sh", &["-c", "printf '[%s]' \"$TERM\""]),
        GEOMETRY,
    )
    .expect("the program starts");
    let output = drain(&runtime);

    let _ = runtime.wait().expect("reap the child");
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
