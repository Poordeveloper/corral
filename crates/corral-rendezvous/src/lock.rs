use std::fs::{File, TryLockError};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rustix::fs::{Mode, OFlags};

use crate::error::RendezvousError;

/// How often a bounded claim retries. Short enough that a daemon started to
/// replace one that is exiting does not idle behind it for long.
const CLAIM_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// A held exclusive claim on the canonical singleton lock.
///
/// The flock — not the socket file — is the singleton truth. It is released by
/// the kernel when this file descriptor closes, which is why an abrupt death
/// needs no cleanup protocol and why PID files are not used (ADR 0001 D2).
#[derive(Debug)]
pub struct SingletonClaim {
    path: PathBuf,
    // Held for the daemon's lifetime; closing releases the claim.
    _file: File,
}

/// What the shared-lock probe found.
///
/// It answers exactly one question — does a canonical primary lock owner exist
/// right now — and nothing about reachability or readiness (ADR 0001 D3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnerProbe {
    OwnerPresent,
    NoOwner,
}

impl SingletonClaim {
    /// Claim the canonical lock, waiting at most `wait` for a departing owner.
    ///
    /// `Ok(None)` means another process held the claim for the whole wait: a
    /// lost race, not a failure. The bounded wait is what keeps a transient
    /// probe from being mistaken for a second primary.
    pub fn acquire(path: &Path, wait: Duration) -> Result<Option<Self>, RendezvousError> {
        let file = open_lock_file(path)?;
        let deadline = Instant::now() + wait;

        loop {
            match file.try_lock() {
                Ok(()) => {
                    return Ok(Some(Self {
                        path: path.to_path_buf(),
                        _file: file,
                    }));
                }
                Err(TryLockError::WouldBlock) => {}
                Err(TryLockError::Error(source)) => {
                    return Err(RendezvousError::Lock {
                        path: path.to_path_buf(),
                        source,
                    });
                }
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            std::thread::sleep(CLAIM_POLL_INTERVAL);
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Ask whether a canonical primary daemon holds the lock right now.
///
/// Success is permission to attempt activation, never ownership: two clients
/// may both see no owner and both spawn, and the daemons then race the
/// exclusive claim (ADR 0001 D3).
pub fn probe_owner(path: &Path) -> Result<OwnerProbe, RendezvousError> {
    let file = open_lock_file(path)?;
    match file.try_lock_shared() {
        Ok(()) => {
            // Closing releases the shared lock immediately; holding it any
            // longer would delay the daemon's exclusive claim for no reason.
            drop(file);
            Ok(OwnerProbe::NoOwner)
        }
        Err(TryLockError::WouldBlock) => Ok(OwnerProbe::OwnerPresent),
        // Anything else — a permission or filesystem fault — is a
        // configuration failure, never "a daemon exists" and never a licence
        // to spawn one.
        Err(TryLockError::Error(source)) => Err(RendezvousError::Lock {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Open the lock file without ever following a symlink at the final component.
///
/// The lock file is a stable rendezvous inode: normal operation creates it once
/// and afterwards only acquires and releases the flock. Nothing here unlinks or
/// recreates it, because two inodes would mean two successful exclusive claims.
fn open_lock_file(path: &Path) -> Result<File, RendezvousError> {
    let flags = OFlags::RDWR | OFlags::CREATE | OFlags::CLOEXEC | OFlags::NOFOLLOW;
    let fd = rustix::fs::open(path, flags, Mode::RUSR | Mode::WUSR).map_err(|errno| {
        RendezvousError::Lock {
            path: path.to_path_buf(),
            source: std::io::Error::from(errno),
        }
    })?;
    Ok(File::from(fd))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_scratch::{permission_checks_apply, scratch_dir};

    #[test]
    fn an_unclaimed_lock_probes_as_having_no_owner() {
        let dir = scratch_dir("lock-free");
        let lock = dir.path().join("corrald.lock");

        assert_eq!(probe_owner(&lock).expect("probe"), OwnerProbe::NoOwner);
    }

    #[test]
    fn a_held_claim_probes_as_owner_present() {
        let dir = scratch_dir("lock-held");
        let lock = dir.path().join("corrald.lock");

        let claim = SingletonClaim::acquire(&lock, Duration::from_millis(50))
            .expect("no fault")
            .expect("claimed");

        assert_eq!(probe_owner(&lock).expect("probe"), OwnerProbe::OwnerPresent);
        drop(claim);
        assert_eq!(probe_owner(&lock).expect("probe"), OwnerProbe::NoOwner);
    }

    #[test]
    fn a_second_claim_loses_the_race_rather_than_failing() {
        let dir = scratch_dir("lock-race");
        let lock = dir.path().join("corrald.lock");

        let _winner = SingletonClaim::acquire(&lock, Duration::from_millis(50))
            .expect("no fault")
            .expect("claimed");
        let loser = SingletonClaim::acquire(&lock, Duration::from_millis(50)).expect("no fault");

        assert!(loser.is_none());
    }

    #[test]
    fn a_probe_never_reports_a_permission_fault_as_an_owner() {
        if !permission_checks_apply() {
            return;
        }
        let dir = scratch_dir("lock-eacces");
        let closed = dir.path().join("closed");
        std::fs::create_dir(&closed).expect("create");
        std::fs::set_permissions(&closed, std::os::unix::fs::PermissionsExt::from_mode(0o000))
            .expect("chmod");

        let error = probe_owner(&closed.join("corrald.lock")).expect_err("permission fault");

        std::fs::set_permissions(&closed, std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .expect("restore");
        assert!(matches!(error, RendezvousError::Lock { .. }));
    }
}
