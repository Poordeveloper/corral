//! `corral uninstall`: take Corral off this account in the order that keeps
//! the user's agents whole — their sessions checked, their configuration
//! cleaned by the daemon, the daemon stopped, and only then the files.
//!
//! Every step is fatal on failure and nothing later is attempted, so an
//! installation is either whole or gone: nothing is removed before the daemon
//! has left, and the file steps are idempotent. Nothing here is forced — no
//! `--force`, no SIGKILL, no guess about a session Corral cannot verify.

use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use corral_client::{ClientActivationPolicy, Connection, activate};
use corral_protocol::method::{self, SessionListItem};
use corral_rendezvous::RendezvousPaths;
use rustix::io::Errno;
use rustix::process::{Pid, Signal};

use crate::installation::Installation;

/// The providers this build asks the daemon to withdraw from.
///
/// The daemon is the authority on what it integrates; this list only says
/// what to ask about. The sibling daemon is this build's own and knows
/// exactly these. A daemon that refuses one of them is not that daemon, and
/// the refusal stops the uninstall with its reason rather than leaving an
/// entry behind that names a binary about to be removed.
const PROVIDERS: [&str; 2] = ["claude", "codex"];

/// How long the daemon is given to leave after SIGTERM.
const EXIT_BUDGET: Duration = Duration::from_secs(10);
const EXIT_POLL: Duration = Duration::from_millis(50);

pub async fn run(policy: &ClientActivationPolicy, purge: bool) -> ExitCode {
    let installation = match Installation::of_running_executable() {
        Ok(installation) => installation,
        Err(refused) => return kept(&refused.to_string()),
    };

    // The sibling daemon is the only reader of integration intent and the
    // only mutator of the providers' files (ADR 0013 D1). One that cannot be
    // activated from this installation is an install-integrity fault, not a
    // reason to guess about the user's configuration.
    let mut connection = match activate(policy).await {
        Ok(connection) => connection,
        Err(error) => return kept(&format!("corrald could not be reached: {error}")),
    };

    if let Err(code) = refuse_while_managing(&mut connection).await {
        return code;
    }
    if let Err(code) = withdraw_integrations(&mut connection).await {
        return code;
    }

    let Some(pid) = connection.daemon_pid() else {
        return kept(
            "the running corrald predates `corral uninstall` and did not name its process; \
             stop it yourself, then run uninstall again",
        );
    };
    // The established connection is what keeps the daemon alive until this
    // moment — an established client holds off its idle exit — so the pid it
    // named is the pid it still has.
    match stop_daemon(pid, EXIT_BUDGET, || connection.reap_daemon_if_exited()) {
        Stopped::Exited => println!("stopped corrald (pid {pid})"),
        Stopped::AlreadyGone => println!("corrald (pid {pid}) had already exited"),
        Stopped::DidNotExit => {
            return kept(&format!(
                "corrald (pid {pid}) did not exit within {}s of SIGTERM",
                EXIT_BUDGET.as_secs()
            ));
        }
        Stopped::NoPermission => {
            return kept(&format!("this account may not stop corrald (pid {pid})"));
        }
        Stopped::SignalFailed(errno) => {
            return kept(&format!(
                "corrald (pid {pid}) could not be signalled: {errno}"
            ));
        }
        Stopped::UnusablePid => {
            return kept(&format!(
                "corrald named a pid this platform cannot signal: {pid}"
            ));
        }
    }

    if let Err(reason) = remove_symlink(&installation) {
        return kept(&reason);
    }
    if let Err(reason) = remove_root(&installation) {
        return kept(&reason);
    }

    let root = match RendezvousPaths::canonical() {
        Ok(paths) => paths.root().to_path_buf(),
        Err(error) => {
            eprintln!("corral: the Corral root could not be resolved: {error}");
            return ExitCode::FAILURE;
        }
    };
    if purge {
        if let Err(source) = remove_tree(&root) {
            eprintln!("corral: {} could not be removed: {source}", root.display());
            return ExitCode::FAILURE;
        }
        println!("removed {}", root.display());
    } else {
        println!(
            "kept {} (corral uninstall --purge removes it)",
            root.display()
        );
    }
    ExitCode::SUCCESS
}

/// Nothing was removed, and this is why.
fn kept(reason: &str) -> ExitCode {
    eprintln!("corral: {reason}; nothing was removed");
    ExitCode::FAILURE
}

/// What the daemon says it is still managing.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct StillManaged {
    pub running: usize,
    pub unknown: usize,
}

impl StillManaged {
    fn is_empty(&self) -> bool {
        self.running == 0 && self.unknown == 0
    }
}

/// Count the managed runs that stand in the way: those the daemon says are
/// running, and those it cannot make a claim about.
///
/// Only rows the daemon started count — a history row is unknown by
/// definition and is not Corral's to end, and a discovered runtime is the
/// user's own. A managed row whose state this build has no word for is
/// unknown, never exited; so is a row this build cannot read at all.
pub fn still_managed(sessions: &[serde_json::Value]) -> StillManaged {
    let mut counted = StillManaged::default();
    for value in sessions {
        let Ok(item) = serde_json::from_value::<SessionListItem>(value.clone()) else {
            counted.unknown += 1;
            continue;
        };
        if item.origin.as_deref() != Some(method::ORIGIN_MANAGED) {
            continue;
        }
        match item.execution_state.as_str() {
            "running" => counted.running += 1,
            "exited" => {}
            _ => counted.unknown += 1,
        }
    }
    counted
}

async fn refuse_while_managing(connection: &mut Connection) -> Result<(), ExitCode> {
    let listed = match connection.session_list().await {
        Ok(listed) => listed,
        Err(error) => return Err(kept(&error.to_string())),
    };
    let counted = still_managed(&listed.sessions);
    if counted.is_empty() {
        return Ok(());
    }
    if counted.running > 0 {
        eprintln!(
            "corral: Corral is still managing {} running session{}",
            counted.running,
            if counted.running == 1 { "" } else { "s" }
        );
    }
    if counted.unknown > 0 {
        eprintln!(
            "corral: could not verify whether {} managed session{} ended",
            counted.unknown,
            if counted.unknown == 1 {
                " has"
            } else {
                "s have"
            }
        );
    }
    eprintln!("End them (corral list), then run uninstall again; nothing was removed.");
    Err(ExitCode::FAILURE)
}

/// Ask the daemon to take its entries out of every provider's configuration,
/// and read back that it did.
///
/// Disabling records the user's withdrawal and removes the entries; the
/// status read afterwards is a second, independent look at the file, and it
/// is what has to say not-installed before the binary those entries name is
/// removed. Withdrawing from a provider that was never enabled records a
/// decision the user is making anyway and writes nothing.
async fn withdraw_integrations(connection: &mut Connection) -> Result<(), ExitCode> {
    for provider in PROVIDERS {
        let disabled = match connection
            .integration(method::INTEGRATION_DISABLE, provider)
            .await
        {
            Ok(answer) => answer,
            Err(error) => return Err(kept(&error.to_string())),
        };
        let standing = match connection
            .integration(method::INTEGRATION_STATUS, provider)
            .await
        {
            Ok(answer) => answer,
            Err(error) => return Err(kept(&error.to_string())),
        };
        if standing.standing != method::STANDING_NOT_INSTALLED {
            let mut reason = format!("{provider} · {}", crate::describe(&standing.standing));
            if let Some(path) = standing.path.as_ref().or(disabled.path.as_ref()) {
                reason.push_str(&format!(" in {path}"));
            }
            if let Some(detail) = disabled.detail.as_ref().or(standing.detail.as_ref()) {
                reason.push_str(&format!(": {detail}"));
            }
            reason.push_str(" — Corral's entries were not withdrawn");
            return Err(kept(&reason));
        }
    }
    Ok(())
}

/// What became of the daemon after SIGTERM.
#[derive(Debug, PartialEq, Eq)]
pub enum Stopped {
    /// Gone within the budget.
    Exited,
    /// No such process: it left on its own between `hello` and the signal.
    AlreadyGone,
    /// Alive when the budget ran out. Never escalated: a daemon that will not
    /// leave is one somebody has to look at, and SIGKILL hangs up every
    /// session it is still running.
    DidNotExit,
    /// This account may not signal that process.
    NoPermission,
    /// A `kill` failure other than the two above.
    SignalFailed(Errno),
    /// A pid outside what this platform can name.
    UnusablePid,
}

/// SIGTERM, then wait for the pid to be gone.
///
/// `reap` runs before every liveness probe: a daemon this process started is
/// its child, and a child that exited answers the probe as alive until it is
/// waited for. Synchronous, because a caller that has just asked its only
/// daemon to leave has nothing else in flight.
pub fn stop_daemon(pid: u32, budget: Duration, mut reap: impl FnMut()) -> Stopped {
    let Some(target) = i32::try_from(pid).ok().and_then(Pid::from_raw) else {
        return Stopped::UnusablePid;
    };
    match rustix::process::kill_process(target, Signal::TERM) {
        Ok(()) => {}
        Err(Errno::SRCH) => return Stopped::AlreadyGone,
        Err(Errno::PERM) => return Stopped::NoPermission,
        Err(errno) => return Stopped::SignalFailed(errno),
    }

    let deadline = Instant::now() + budget;
    loop {
        reap();
        if rustix::process::test_kill_process(target) == Err(Errno::SRCH) {
            return Stopped::Exited;
        }
        if Instant::now() >= deadline {
            return Stopped::DidNotExit;
        }
        std::thread::sleep(EXIT_POLL);
    }
}

/// Take away `~/.local/bin/corral` if it is ours: a link that resolves into
/// this installation. Anything else at that name is left where it is and
/// said so.
fn remove_symlink(installation: &Installation) -> Result<(), String> {
    let link = installation.symlink();
    match std::fs::symlink_metadata(link) {
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(format!(
                "{} could not be examined: {source}",
                link.display()
            ));
        }
        Ok(metadata) if !metadata.file_type().is_symlink() => {
            println!("kept {}: it is not a symlink", link.display());
            return Ok(());
        }
        Ok(_) => {}
    }
    // Resolved through the link, the way a shell runs it. The installation
    // is still on disk at this step, which is why the link goes first.
    let ours = std::fs::canonicalize(link).is_ok_and(|target| target == installation.corral());
    if !ours {
        println!(
            "kept {}: it does not point into this installation",
            link.display()
        );
        return Ok(());
    }
    std::fs::remove_file(link)
        .map_err(|source| format!("{} could not be removed: {source}", link.display()))?;
    println!("removed {}", link.display());
    Ok(())
}

fn remove_root(installation: &Installation) -> Result<(), String> {
    let root = installation.root();
    remove_tree(root)
        .map_err(|source| format!("{} could not be removed: {source}", root.display()))?;
    println!("removed {}", root.display());
    Ok(())
}

/// Remove a directory tree, treating one already gone as done.
fn remove_tree(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_dir_all(path) {
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        outcome => outcome,
    }
}

#[cfg(test)]
#[path = "uninstall_tests.rs"]
mod tests;
