//! End-to-end: `corral uninstall`, from an installation laid out the way the
//! installer lays one out, against the daemon it activates as its sibling.
//!
//! What is real: the installation on disk, the `PATH` link, activation, the
//! daemon's own integration writes into the user's `settings.json`, SIGTERM
//! and the wait for the pid. What a stand-in daemon supplies is only the two
//! answers a real one cannot be made to give on demand — a managed run in the
//! unverifiable state, and a hello from before `pid` existed.

// The repository allows unwrap/expect in tests; that setting does not reach
// helpers that sit outside a `#[test]` function in an integration-test crate.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod support;

use std::process::{Output, Stdio};
use std::time::Duration;

use serde_json::{Value, json};
use support::installation::{self, Installed};
use support::wire::{FakeBehaviour, hello_result, spawn_fake_daemon};
use support::{TestAccount, lock_is_held, run, stderr, stdout};

/// `corral uninstall` the way a person runs it: through the `PATH` link.
fn uninstall(account: &TestAccount, installed: &Installed, purge: bool) -> Output {
    let mut command = account.command(&installed.symlink);
    command.arg("uninstall");
    if purge {
        command.arg("--purge");
    }
    run(command.stdin(Stdio::null()))
}

fn settings(account: &TestAccount) -> String {
    std::fs::read_to_string(account.user_home().join(".claude/settings.json")).unwrap_or_default()
}

fn managed_row(execution_state: &str) -> Value {
    json!({
        "session_id": "0f9b6c1a-7d2e-4a55-9c31-000000000000",
        "title": "sh",
        "execution_state": execution_state,
        "origin": "managed",
    })
}

fn not_installed() -> Value {
    json!({"result": {
        "provider": "claude",
        "standing": "not-installed",
        "claims_delivery": false,
    }})
}

/// The staged binary lives under `target/`, which is not a shape: refused by
/// name, and before any daemon is started to be asked about.
#[test]
fn a_binary_that_is_not_installed_is_refused_by_name() {
    let account = TestAccount::new("uninst-elsewhere");

    let output = run(account.corral().arg("uninstall").stdin(Stdio::null()));

    let words = stderr(&output);
    assert!(!output.status.success());
    assert!(
        words.contains("is not an installed Corral") && words.contains("nothing was removed"),
        "{words}"
    );
    assert!(
        !lock_is_held(&account.lock()) && !account.socket().exists(),
        "no daemon is started on the way to a refusal"
    );
}

/// A run Corral started and is still running is the user's work; uninstall
/// refuses, names the count, and leaves the daemon and the files alone.
#[test]
fn uninstall_refuses_while_a_managed_session_is_running() {
    let account = TestAccount::new("uninst-running");
    let installed = installation::install(&account);
    let started = run(account
        .command(&installed.symlink)
        .args(["new", "--", "/bin/sleep", "30"])
        .stdin(Stdio::null()));
    assert!(started.status.success(), "{}", stderr(&started));

    let output = uninstall(&account, &installed, false);

    let words = stderr(&output);
    assert!(!output.status.success());
    assert!(
        words.contains("still managing 1 running session") && words.contains("nothing was removed"),
        "{words}"
    );
    assert!(installed.is_intact());
    assert!(lock_is_held(&account.lock()), "the daemon was stopped");
}

/// A managed run the daemon cannot make a claim about is not one that ended.
#[test]
fn uninstall_refuses_when_a_managed_run_cannot_be_verified() {
    let account =
        TestAccount::new("uninst-unknown").with_activation_deadline(Duration::from_secs(3));
    let installed = installation::install(&account);
    let fake = spawn_fake_daemon(
        &account.socket(),
        FakeBehaviour::Scripted {
            hello: hello_result(None),
            answers: vec![(
                "session.list".to_owned(),
                json!({"result": {"sessions": [managed_row("unknown")]}}),
            )],
        },
    );

    let output = uninstall(&account, &installed, false);

    let words = stderr(&output);
    assert!(!output.status.success());
    assert!(
        words.contains("could not verify whether 1 managed session has ended")
            && words.contains("nothing was removed"),
        "{words}"
    );
    assert!(installed.is_intact());
    assert_eq!(
        fake.methods(),
        vec!["session.list".to_owned()],
        "nothing was withdrawn on the way to a refusal"
    );
}

/// A daemon from before `hello` named a pid cannot be stopped without
/// guessing, and uninstall never guesses: it stops after the integration
/// cleanup and before the first file is removed.
#[test]
fn uninstall_stops_before_removing_anything_when_the_daemon_does_not_name_its_process() {
    let account = TestAccount::new("uninst-nopid").with_activation_deadline(Duration::from_secs(3));
    let installed = installation::install(&account);
    let fake = spawn_fake_daemon(
        &account.socket(),
        FakeBehaviour::Scripted {
            hello: hello_result(None),
            answers: vec![
                (
                    "session.list".to_owned(),
                    json!({"result": {"sessions": []}}),
                ),
                ("integration.disable".to_owned(), not_installed()),
                ("integration.status".to_owned(), not_installed()),
            ],
        },
    );

    let output = uninstall(&account, &installed, false);

    let words = stderr(&output);
    assert!(!output.status.success());
    assert!(
        words.contains("did not name its process") && words.contains("nothing was removed"),
        "{words}"
    );
    assert!(installed.is_intact());
    assert_eq!(
        fake.methods(),
        vec![
            "session.list",
            "integration.disable",
            "integration.status",
            "integration.disable",
            "integration.status",
        ],
        "every provider is withdrawn and read back before the daemon is asked to stop"
    );
}

/// The whole path: an enabled integration is taken back out by the daemon,
/// the daemon leaves on SIGTERM, the link and the installation go, and the
/// Corral root stays.
#[test]
fn uninstall_removes_the_installation_and_keeps_the_corral_root() {
    let account = TestAccount::new("uninst-clean");
    let installed = installation::install(&account);
    let enabled = run(account
        .command(&installed.symlink)
        .args(["integration", "enable", "claude"])
        .stdin(Stdio::null()));
    assert!(enabled.status.success(), "{}", stderr(&enabled));
    assert!(
        settings(&account).contains("hook-relay"),
        "the integration was not installed: {}",
        settings(&account)
    );

    let output = uninstall(&account, &installed, false);

    assert!(output.status.success(), "{}", stderr(&output));
    let said = stdout(&output);
    assert!(said.contains("stopped corrald (pid "), "{said}");
    assert!(
        !settings(&account).contains("hook-relay"),
        "Corral's entries outlived the binary they name: {}",
        settings(&account)
    );
    assert!(
        std::fs::symlink_metadata(&installed.symlink).is_err(),
        "the PATH link outlived the installation"
    );
    assert!(!installed.root.exists(), "the installation was not removed");
    assert!(!lock_is_held(&account.lock()), "the daemon did not leave");
    assert!(!account.socket().exists(), "the daemon left its endpoint");
    assert!(account.registry().is_file(), "the Corral root was not kept");
    assert!(said.contains("kept"), "{said}");
}

#[test]
fn purge_removes_the_corral_root_last() {
    let account = TestAccount::new("uninst-purge");
    let installed = installation::install(&account);

    let output = uninstall(&account, &installed, true);

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(!installed.root.exists());
    assert!(
        !account.corral_root().exists(),
        "--purge left the Corral root"
    );
}

/// A link at `~/.local/bin/corral` that is somebody else's is not ours to
/// remove, and saying so is not a failure.
#[test]
fn a_link_that_points_elsewhere_is_kept() {
    let account = TestAccount::new("uninst-link");
    let installed = installation::install(&account);
    std::fs::remove_file(&installed.symlink).expect("replace the link");
    std::os::unix::fs::symlink("/usr/bin/true", &installed.symlink).expect("another link");

    let output = run(account
        .command(&installed.corral)
        .arg("uninstall")
        .stdin(Stdio::null()));

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        std::fs::read_link(&installed.symlink).is_ok(),
        "a link into somewhere else was removed"
    );
    assert!(!installed.root.exists());
    assert!(
        stdout(&output).contains("does not point into this installation"),
        "{}",
        stdout(&output)
    );
}
