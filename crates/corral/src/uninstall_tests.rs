use super::*;

use std::process::{Child, Command, Stdio};

use serde_json::{Value, json};

fn row(origin: Option<&str>, execution_state: &str) -> Value {
    let mut row = json!({
        "session_id": "0f9b6c1a-7d2e-4a55-9c31-000000000000",
        "title": "sh",
        "execution_state": execution_state,
    });
    if let Some(origin) = origin {
        row["origin"] = json!(origin);
    }
    row
}

#[test]
fn managed_runs_that_are_running_or_unverifiable_stand_in_the_way() {
    let counted = still_managed(&[
        row(Some("managed"), "running"),
        row(Some("managed"), "unknown"),
        row(Some("managed"), "exited"),
        row(Some("managed"), "running"),
    ]);
    assert_eq!(
        counted,
        StillManaged {
            running: 2,
            unknown: 1
        }
    );
}

/// A history row is unknown by definition and not Corral's to end; a
/// discovered runtime is the user's own; a row with no origin is one Corral
/// does not reliably know the origin of, and a guessed "managed" would refuse
/// over a session this daemon never started.
#[test]
fn only_rows_the_daemon_started_count() {
    let counted = still_managed(&[
        row(Some("history"), "unknown"),
        row(Some("discovered"), "running"),
        row(None, "running"),
    ]);
    assert_eq!(counted, StillManaged::default());
}

/// A state this build has no word for is unknown, never exited; a row this
/// build cannot read at all is a session it cannot verify.
#[test]
fn what_this_build_cannot_read_is_unverified_not_ended() {
    let counted = still_managed(&[
        row(Some("managed"), "hibernating"),
        json!({"origin": "managed"}),
    ]);
    assert_eq!(
        counted,
        StillManaged {
            running: 0,
            unknown: 2
        }
    );
}

fn shell(script: &str) -> Child {
    Command::new("/bin/sh")
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start a child")
}

fn alive(child: &Child) -> bool {
    let pid = Pid::from_raw(i32::try_from(child.id()).expect("a pid")).expect("a live pid");
    rustix::process::test_kill_process(pid).is_ok()
}

/// A process that ignores SIGTERM is reported, not escalated: it is still
/// alive afterwards, which is the whole of the "never SIGKILL" rule.
#[test]
fn a_process_that_ignores_sigterm_is_reported_and_left_alive() {
    // The signal must not race the trap: a shell that has not yet run
    // `trap` dies of SIGTERM like anything else, so it says when it has.
    let armed = std::env::temp_dir().join(format!("corral-ignores-term-{}", std::process::id()));
    let _ = std::fs::remove_file(&armed);
    let mut child = shell(&format!(
        "trap '' TERM; : > '{}'; sleep 30",
        armed.display()
    ));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !armed.exists() {
        assert!(Instant::now() < deadline, "the shell never armed its trap");
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = std::fs::remove_file(&armed);

    let outcome = stop_daemon(child.id(), Duration::from_millis(300), || {
        let _ = child.try_wait();
    });

    assert_eq!(outcome, Stopped::DidNotExit);
    assert!(alive(&child), "stop_daemon must never escalate");
    let _ = child.kill();
    let _ = child.wait();
}

/// A child that exits on SIGTERM is a zombie until it is waited for, and
/// the pid of a zombie still answers a liveness probe. `reap` is what turns
/// that into an exit within the budget.
#[test]
fn a_process_that_exits_on_sigterm_is_reaped_and_reported_gone() {
    let mut child = shell("sleep 30");

    let outcome = stop_daemon(child.id(), Duration::from_secs(5), || {
        let _ = child.try_wait();
    });

    assert_eq!(outcome, Stopped::Exited);
    assert!(child.try_wait().expect("waitable").is_some());
}

#[test]
fn a_process_already_gone_is_not_an_error() {
    let mut child = shell("exit 0");
    child.wait().expect("reap");

    assert_eq!(
        stop_daemon(child.id(), Duration::from_secs(1), || {}),
        Stopped::AlreadyGone
    );
}

#[test]
fn a_pid_this_platform_cannot_name_is_refused() {
    assert_eq!(
        stop_daemon(u32::MAX, Duration::from_secs(1), || {}),
        Stopped::UnusablePid
    );
}
