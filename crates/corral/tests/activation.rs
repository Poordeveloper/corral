//! Activation: who may start this account's daemon, and where.

// The repository allows unwrap/expect in tests; that setting does not reach
// helpers that sit outside a `#[test]` function in an integration-test crate.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod support;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

use support::{SETTLE, TestAccount, lock_is_held, run, stderr, stdout, wait_until};

#[test]
fn a_cold_start_activates_a_daemon_and_answers() {
    let account = TestAccount::new("cold-start");

    let output = run(account.corral().arg("ping"));

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("protocol"), "{}", stdout(&output));
    assert!(account.socket().exists());
    assert!(lock_is_held(&account.lock()));
}

#[test]
fn the_empty_session_list_is_reported_as_a_fact() {
    let account = TestAccount::new("empty-list");

    let output = run(account.corral().arg("list"));

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output).trim(), "No sessions.");
}

/// The canonical rendezvous follows the OS account, so the environment a
/// command happens to inherit cannot split one account into two daemons.
#[test]
fn the_environment_cannot_move_the_rendezvous() {
    let account = TestAccount::new("environment");
    let decoy_home = account.scratch().join("decoy-home");
    let decoy_runtime = account.scratch().join("decoy-runtime");
    std::fs::create_dir_all(&decoy_home).expect("create");
    std::fs::create_dir_all(&decoy_runtime).expect("create");

    let first = run(account
        .corral()
        .arg("ping")
        .env("HOME", &decoy_home)
        .env("XDG_RUNTIME_DIR", &decoy_runtime));
    let second = run(account
        .corral()
        .arg("ping")
        .env("HOME", account.scratch().join("another-home"))
        .env_remove("XDG_RUNTIME_DIR"));

    assert!(first.status.success(), "{}", stderr(&first));
    assert!(second.status.success(), "{}", stderr(&second));
    assert!(account.socket().exists());
    assert!(!decoy_home.join(".corral").exists(), "HOME took part");
    assert!(
        !decoy_runtime.join("corrald.sock").exists(),
        "XDG_RUNTIME_DIR took part"
    );
}

#[test]
fn concurrent_cold_starts_converge_on_one_daemon() {
    let account = TestAccount::new("concurrent");

    let clients: Vec<_> = (0..5)
        .map(|_| {
            account
                .corral()
                .arg("ping")
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("start corral")
        })
        .collect();

    for client in clients {
        let output = client.wait_with_output().expect("wait for corral");
        assert!(output.status.success(), "{}", stderr(&output));
    }
    assert!(account.socket().exists());
    // Losers of the singleton race exit without serving, so exactly one daemon
    // is left holding the claim.
    assert!(lock_is_held(&account.lock()));
}

#[test]
fn an_endpoint_override_never_starts_a_daemon() {
    let account = TestAccount::new("override-dead");
    let elsewhere = account.scratch().join("elsewhere.sock");

    let output = run(account
        .corral()
        .arg("ping")
        .env("CORRAL_ENDPOINT", &elsewhere));

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("CORRAL_ENDPOINT"),
        "{}",
        stderr(&output)
    );
    assert!(
        !account.socket().exists(),
        "an override must not fall back to the canonical rendezvous"
    );
}

#[test]
fn an_override_does_not_fall_back_to_a_live_canonical_daemon() {
    let account = TestAccount::new("override-no-fallback");
    let started = run(account.corral().arg("ping"));
    assert!(started.status.success(), "{}", stderr(&started));

    let output = run(account
        .corral()
        .arg("ping")
        .env("CORRAL_ENDPOINT", account.scratch().join("missing.sock")));

    assert!(!output.status.success(), "{}", stdout(&output));
}

#[test]
fn a_malformed_override_is_terminal() {
    let account = TestAccount::new("override-malformed");

    let output = run(account
        .corral()
        .arg("ping")
        .env("CORRAL_ENDPOINT", "run/corrald.sock"));

    assert!(!output.status.success());
    assert!(stderr(&output).contains("absolute"), "{}", stderr(&output));
    assert!(!account.socket().exists());
}

#[test]
fn a_stale_socket_left_by_a_dead_daemon_is_replaced() {
    let account = TestAccount::new("stale-socket");
    std::fs::create_dir_all(account.socket().parent().expect("run dir")).expect("create");
    let listener = std::os::unix::net::UnixListener::bind(account.socket()).expect("bind");
    drop(listener);
    assert!(account.socket().exists());

    let output = run(account.corral().arg("ping"));

    assert!(output.status.success(), "{}", stderr(&output));
}

/// Cleaning a stale rendezvous must never become a file-deletion primitive.
#[test]
fn a_regular_file_at_the_endpoint_is_never_deleted() {
    let account =
        TestAccount::new("occupied-endpoint").with_activation_deadline(Duration::from_secs(3));
    std::fs::create_dir_all(account.socket().parent().expect("run dir")).expect("create");
    std::fs::write(account.socket(), b"not a socket").expect("write");

    let output = run(account.corral().arg("ping"));

    assert!(!output.status.success());
    assert_eq!(
        std::fs::read(account.socket()).expect("still there"),
        b"not a socket"
    );
}

/// A held lock with no reachable endpoint is not something a client may repair
/// by starting a second daemon.
#[test]
fn a_wedged_rendezvous_reports_the_owner_rather_than_starting_a_rival() {
    let account = TestAccount::new("wedged").with_activation_deadline(Duration::from_millis(400));
    std::fs::create_dir_all(account.lock().parent().expect("run dir")).expect("create");
    let held = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(account.lock())
        .expect("open the lock");
    held.lock().expect("hold the claim");

    let output = run(account.corral().arg("ping"));

    assert!(!output.status.success());
    let message = stderr(&output);
    assert!(message.contains("holds"), "{message}");
    assert!(!account.socket().exists(), "{message}");
    drop(held);
}

#[test]
fn a_permission_fault_is_never_reported_as_a_running_daemon() {
    if rustix::process::geteuid().is_root() {
        return;
    }
    let account = TestAccount::new("permission");
    let run_dir = account.socket().parent().expect("run dir").to_path_buf();
    std::fs::create_dir_all(&run_dir).expect("create");
    std::fs::set_permissions(&run_dir, PermissionsExt::from_mode(0o000)).expect("chmod");

    let output = run(account.corral().arg("ping"));

    std::fs::set_permissions(&run_dir, PermissionsExt::from_mode(0o700)).expect("restore");
    assert!(!output.status.success());
    let message = stderr(&output);
    assert!(
        !message.contains("holds"),
        "a permission fault must not be reported as an owner: {message}"
    );
}

/// The daemon is resolved beside the running executable and nowhere else, so a
/// shell's `PATH` cannot decide which daemon an account talks to.
#[test]
fn only_the_sibling_daemon_may_be_started() {
    let account = TestAccount::new("sibling-only");
    let install = account.scratch().join("install");
    let decoys = account.scratch().join("decoys");
    std::fs::create_dir_all(&install).expect("create");
    std::fs::create_dir_all(&decoys).expect("create");
    std::fs::copy(support::CORRAL_BINARY, install.join("corral")).expect("copy corral");
    write_decoy(
        &decoys.join("corrald"),
        &account.scratch().join("decoy-ran"),
    );

    let output = run(std::process::Command::new(install.join("corral"))
        .arg("ping")
        .env("CORRAL_TEST_ROOT", account.corral_root())
        .env("CORRAL_TEST_ACTIVATION_DEADLINE_MS", "2000")
        .env_remove("CORRAL_ENDPOINT")
        .env("PATH", &decoys));

    assert!(!output.status.success());
    assert!(stderr(&output).contains("reinstall"), "{}", stderr(&output));
    assert!(
        !account.scratch().join("decoy-ran").exists(),
        "a daemon on PATH must never be started"
    );
}

#[test]
fn an_unwritable_log_destination_does_not_stop_the_daemon() {
    let account = TestAccount::new("log-blocked");
    // A regular file where the log directory belongs: creating the directory
    // fails, and logging is not a correctness authority.
    std::fs::write(account.log_dir(), b"blocked").expect("write");

    let output = run(account.corral().arg("ping"));

    assert!(output.status.success(), "{}", stderr(&output));
    wait_until(SETTLE, || account.socket().exists());
}

fn write_decoy(path: &Path, marker: &Path) {
    std::fs::write(
        path,
        format!("#!/bin/sh\ntouch {}\nsleep 30\n", marker.display()),
    )
    .expect("write the decoy");
    std::fs::set_permissions(path, PermissionsExt::from_mode(0o755)).expect("chmod the decoy");
}

/// A client that arrives while an owner is on its way out must converge, not
/// fail: the whole activation budget is one deadline covering probe, spawn,
/// connect and handshake together.
#[test]
fn a_client_waits_out_a_departing_owner_and_then_starts_one() {
    let account = TestAccount::new("departing").with_activation_deadline(Duration::from_secs(10));
    std::fs::create_dir_all(account.lock().parent().expect("run dir")).expect("create");
    let held = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(account.lock())
        .expect("open the lock");
    held.lock().expect("hold the claim");

    let client = account
        .corral()
        .arg("ping")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("start corral");

    // While the claim is held the client must not have started anything.
    std::thread::sleep(Duration::from_millis(300));
    assert!(!account.socket().exists());
    drop(held);

    let output = client.wait_with_output().expect("wait for corral");
    assert!(output.status.success(), "{}", stderr(&output));
}
