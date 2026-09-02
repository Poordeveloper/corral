use super::*;

/// The process this test runs in is the one process that is certainly there,
/// certainly this account's, and whose answers can be checked against what
/// the standard library already knows.
#[cfg(target_os = "linux")]
#[test]
fn this_process_identifies_itself() {
    let Observation::Identified(identity) = observe(std::process::id()) else {
        panic!("a process cannot fail to observe itself");
    };

    assert_eq!(identity.pid, std::process::id());
    assert_eq!(identity.parent, std::os::unix::process::parent_id());
    assert_eq!(
        identity.group,
        rustix::process::getpgrp().as_raw_pid().unsigned_abs()
    );
    assert!(identity.executable.is_absolute());
    assert!(
        identity.started <= SystemTime::now(),
        "a process did not start after the moment it was asked about",
    );
}

/// The executable, not the path the process was invoked by. Measured on both
/// platforms and both provider install channels: the invoked path is a
/// symlink or a launcher and the real binary is elsewhere.
#[cfg(target_os = "linux")]
#[test]
fn the_executable_is_the_one_actually_running() {
    let Observation::Identified(identity) = observe(std::process::id()) else {
        panic!("a process cannot fail to observe itself");
    };

    let expected = std::env::current_exe().expect("this test binary is locatable");
    assert_eq!(
        identity.executable.canonicalize().ok(),
        expected.canonicalize().ok(),
    );
}

/// The distinction the claim ladder rests on: only `Gone` supports concluding
/// that a Run ended, and a permission failure must never reach it.
#[cfg(target_os = "linux")]
#[test]
fn a_pid_that_is_not_there_is_gone_rather_than_unreadable() {
    let absent = (1..u32::MAX)
        .rev()
        .take(64)
        .find(|pid| matches!(observe(*pid), Observation::Gone));

    assert!(
        absent.is_some(),
        "some pid in the top of the range is unused",
    );
}

/// Until macOS observation is decided, this build says it cannot look rather
/// than answering something it did not observe. Unknown is a first-class
/// state and never collapses into `Gone`, so nothing here can be read as a
/// process that ended.
#[cfg(target_os = "macos")]
#[test]
fn this_build_says_it_cannot_observe_rather_than_guessing() {
    assert_eq!(observe(std::process::id()), Observation::Unobservable);
    assert_eq!(observe(1), Observation::Unobservable);
    assert_ne!(observe(u32::MAX - 1), Observation::Gone);
}

/// pid 0 is the kernel's placeholder for "no parent", not a process. Saying
/// it is gone would claim a process ended that never existed.
#[test]
fn the_placeholder_parent_is_not_a_process_that_ended() {
    assert_eq!(observe(0), Observation::Unobservable);
}
