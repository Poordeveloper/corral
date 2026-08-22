use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use corral_rendezvous::RendezvousPaths;
use tokio::process::{Child, Command};

use crate::error::{ActivationError, SpawnOutcome};

/// How auto-activation tells `corrald` it was started by a client rather than
/// run by a person. An internal marker, not a stable command-line contract.
pub const AUTO_START_FLAG: &str = "--internal-auto-start";

const DAEMON_BINARY: &str = "corrald";

/// A daemon this client started.
///
/// The handle is retained for as long as the surface lives so the child is
/// reaped rather than left as a zombie. It is never a parent-child lifetime
/// relationship: the daemon detaches itself and outlives this process.
#[derive(Debug)]
pub struct SpawnedDaemon {
    child: Child,
    pid: u32,
}

impl SpawnedDaemon {
    pub fn outcome(&mut self) -> SpawnOutcome {
        let exit_code = match self.child.try_wait() {
            Ok(Some(status)) => Some(status.code().unwrap_or(-1)),
            _ => None,
        };
        SpawnOutcome {
            pid: self.pid,
            exit_code,
        }
    }
}

/// Resolve `corrald` as the sibling of the running executable.
///
/// Sibling-only, because a shell-local `PATH` must not decide which daemon
/// binary an entire OS account talks to. The real location is resolved first so
/// that a symlinked launcher finds the daemon next to the actual install.
pub fn sibling_daemon() -> Result<PathBuf, ActivationError> {
    let running = std::env::current_exe().map_err(|source| ActivationError::InstallIntegrity {
        expected: PathBuf::from(DAEMON_BINARY),
        detail: format!("the running executable could not be located: {source}"),
    })?;
    let real = running.canonicalize().unwrap_or(running);
    let directory = real
        .parent()
        .ok_or_else(|| ActivationError::InstallIntegrity {
            expected: real.clone(),
            detail: "the running executable has no directory".to_owned(),
        })?;
    let daemon = directory.join(DAEMON_BINARY);

    let metadata =
        std::fs::metadata(&daemon).map_err(|source| ActivationError::InstallIntegrity {
            expected: daemon.clone(),
            detail: source.to_string(),
        })?;
    if !metadata.is_file() {
        return Err(ActivationError::InstallIntegrity {
            expected: daemon,
            detail: "it is not a regular file".to_owned(),
        });
    }
    if metadata.permissions().mode() & 0o111 == 0 {
        return Err(ActivationError::InstallIntegrity {
            expected: daemon,
            detail: "it is not executable".to_owned(),
        });
    }
    Ok(daemon)
}

/// Start the sibling daemon in auto-start mode.
///
/// Standard output is discarded and standard error is pointed at the daemon
/// log; failing to open that log changes nothing about ownership or readiness,
/// so it degrades to a null sink instead of failing activation.
pub fn spawn_daemon(paths: &RendezvousPaths) -> Result<SpawnedDaemon, ActivationError> {
    let program = sibling_daemon()?;

    let child = Command::new(&program)
        .arg(AUTO_START_FLAG)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(daemon_log_sink(paths))
        .spawn()
        .map_err(|source| ActivationError::Spawn {
            program: program.clone(),
            source,
        })?;

    let pid = child.id().unwrap_or_default();
    Ok(SpawnedDaemon { child, pid })
}

fn daemon_log_sink(paths: &RendezvousPaths) -> Stdio {
    if paths.ensure_log_dir().is_err() {
        return Stdio::null();
    }
    open_append(paths.log_file()).map_or_else(Stdio::null, Stdio::from)
}

fn open_append(path: &Path) -> Option<std::fs::File> {
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
        .ok()
}
