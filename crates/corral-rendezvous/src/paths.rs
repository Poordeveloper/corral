use std::ffi::OsStr;
use std::fs::DirBuilder;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

use uzers::os::unix::UserExt;

use crate::error::{InvalidEndpointReason, RendezvousError};

const CORRAL_DIR: &str = ".corral";
const RUN_DIR: &str = "run";
const LOG_DIR: &str = "log";
const SOCKET_FILE: &str = "corrald.sock";
const LOCK_FILE: &str = "corrald.lock";
const LOG_FILE: &str = "corrald.log";

/// Runtime and log directories are user-private: the flock and these modes are
/// a transport fence, deliberately not a security boundary (ADR 0001).
const PRIVATE_DIR_MODE: u32 = 0o700;

/// The canonical rendezvous of one OS account.
///
/// Derivation is a pure function of the account home, so every process of the
/// same effective OS user on the same host computes the same paths whatever
/// their shell, session type, cron or ssh environment looks like (ADR 0001 D1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RendezvousPaths {
    run_dir: PathBuf,
    socket: PathBuf,
    lock: PathBuf,
    log_dir: PathBuf,
    log_file: PathBuf,
}

impl RendezvousPaths {
    /// The canonical rendezvous of the effective OS user.
    pub fn canonical() -> Result<Self, RendezvousError> {
        Self::for_account_home(account_home()?)
    }

    /// The same derivation rooted at an explicit account home.
    ///
    /// This is the whole of the path rule; `canonical` only supplies the home.
    pub fn for_account_home(home: impl AsRef<Path>) -> Result<Self, RendezvousError> {
        let base = home.as_ref().join(CORRAL_DIR);
        let run_dir = base.join(RUN_DIR);
        let log_dir = base.join(LOG_DIR);
        let socket = run_dir.join(SOCKET_FILE);

        if socket_address_length_exceeded(&socket) {
            return Err(RendezvousError::CanonicalEndpointTooLong { path: socket });
        }

        Ok(Self {
            lock: run_dir.join(LOCK_FILE),
            log_file: log_dir.join(LOG_FILE),
            socket,
            run_dir,
            log_dir,
        })
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    pub fn lock(&self) -> &Path {
        &self.lock
    }

    pub fn run_dir(&self) -> &Path {
        &self.run_dir
    }

    pub fn log_file(&self) -> &Path {
        &self.log_file
    }

    /// Create the user-private runtime directory. Idempotent.
    pub fn ensure_run_dir(&self) -> Result<(), RendezvousError> {
        ensure_private_dir(&self.run_dir)
    }

    /// Create the user-private log directory. Idempotent.
    pub fn ensure_log_dir(&self) -> Result<(), RendezvousError> {
        ensure_private_dir(&self.log_dir)
    }
}

/// Validate a caller-supplied endpoint path.
///
/// An override redirects a client; it never creates a second primary daemon,
/// so an unusable override is terminal rather than a reason to fall back to
/// the canonical rendezvous (ADR 0001 D1).
pub fn validate_endpoint_path(raw: &OsStr) -> Result<PathBuf, RendezvousError> {
    let invalid = |reason| RendezvousError::InvalidExplicitEndpoint {
        raw: raw.to_os_string(),
        reason,
    };

    if raw.is_empty() {
        return Err(invalid(InvalidEndpointReason::Empty));
    }
    let path = PathBuf::from(raw);
    if !path.is_absolute() {
        return Err(invalid(InvalidEndpointReason::Relative));
    }
    if socket_address_length_exceeded(&path) {
        return Err(invalid(InvalidEndpointReason::TooLong));
    }
    Ok(path)
}

/// Ask the platform itself whether the path can address a Unix socket, rather
/// than hardcoding a `sun_path` size that differs across the supported systems.
///
/// This is the same construction a bind or connect performs, so a path this
/// accepts cannot be rejected later by the code that actually uses it.
fn socket_address_length_exceeded(path: &Path) -> bool {
    std::os::unix::net::SocketAddr::from_pathname(path).is_err()
}

fn ensure_private_dir(path: &Path) -> Result<(), RendezvousError> {
    DirBuilder::new()
        .recursive(true)
        .mode(PRIVATE_DIR_MODE)
        .create(path)
        .map_err(|source| RendezvousError::RuntimeDirectory {
            path: path.to_path_buf(),
            source,
        })?;

    // `recursive` accepts an existing path without inspecting it, so confirm
    // what is actually there before anything binds or locks inside it.
    let metadata = std::fs::metadata(path).map_err(|source| RendezvousError::RuntimeDirectory {
        path: path.to_path_buf(),
        source,
    })?;
    if !metadata.is_dir() {
        return Err(RendezvousError::RuntimeDirectory {
            path: path.to_path_buf(),
            source: std::io::Error::from(std::io::ErrorKind::NotADirectory),
        });
    }
    Ok(())
}

/// The home directory of the effective OS user, from the account database.
///
/// Never `$HOME`: the canonical rendezvous is user-wide, so a shell variable
/// must not be able to give one OS account two primary daemons (ADR 0001 D1).
fn account_home() -> Result<PathBuf, RendezvousError> {
    if let Some(root) = test_account_home() {
        return Ok(root);
    }

    let uid = uzers::get_effective_uid();
    let user =
        uzers::get_user_by_uid(uid).ok_or_else(|| RendezvousError::AccountHomeUnresolvable {
            uid,
            detail: "no account database entry".to_owned(),
        })?;
    let home = user.home_dir().to_path_buf();

    if home.as_os_str().is_empty() {
        return Err(RendezvousError::AccountHomeUnresolvable {
            uid,
            detail: "the account entry has no home directory".to_owned(),
        });
    }
    if !home.is_absolute() {
        return Err(RendezvousError::AccountHomeUnresolvable {
            uid,
            detail: format!("the account home {} is not absolute", home.display()),
        });
    }
    Ok(home)
}

/// Test-support only (ADR 0001, "Test injection").
///
/// Process-level tests must exercise real activation without writing into the
/// developer's own account, and the account database cannot be redirected. The
/// substitution is compiled out of release builds, so production packaging
/// cannot reach it and canonical identity there always comes from the account
/// database.
#[cfg(debug_assertions)]
fn test_account_home() -> Option<PathBuf> {
    std::env::var_os("CORRAL_TEST_ROOT").map(PathBuf::from)
}

#[cfg(not(debug_assertions))]
fn test_account_home() -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_every_artifact_from_the_account_home() {
        let paths = RendezvousPaths::for_account_home("/home/example").expect("derivable");

        assert_eq!(
            paths.socket(),
            Path::new("/home/example/.corral/run/corrald.sock")
        );
        assert_eq!(
            paths.lock(),
            Path::new("/home/example/.corral/run/corrald.lock")
        );
        assert_eq!(paths.run_dir(), Path::new("/home/example/.corral/run"));
        assert_eq!(
            paths.log_file(),
            Path::new("/home/example/.corral/log/corrald.log")
        );
    }

    #[test]
    fn derivation_depends_on_nothing_but_the_home() {
        let a = RendezvousPaths::for_account_home("/home/example").expect("derivable");
        let b = RendezvousPaths::for_account_home("/home/example/").expect("derivable");
        assert_eq!(a, b);
    }

    #[test]
    fn a_canonical_path_too_long_for_a_socket_is_a_configuration_error() {
        let deep = format!("/{}", "d".repeat(200));
        let error = RendezvousPaths::for_account_home(&deep).expect_err("too long");
        assert!(matches!(
            error,
            RendezvousError::CanonicalEndpointTooLong { .. }
        ));
    }

    #[test]
    fn an_empty_override_is_rejected_without_falling_back() {
        let error = validate_endpoint_path(OsStr::new("")).expect_err("empty");
        assert!(matches!(
            error,
            RendezvousError::InvalidExplicitEndpoint {
                reason: InvalidEndpointReason::Empty,
                ..
            }
        ));
    }

    #[test]
    fn a_relative_override_is_rejected_without_falling_back() {
        let error = validate_endpoint_path(OsStr::new("run/corrald.sock")).expect_err("relative");
        assert!(matches!(
            error,
            RendezvousError::InvalidExplicitEndpoint {
                reason: InvalidEndpointReason::Relative,
                ..
            }
        ));
    }

    #[test]
    fn an_oversized_override_is_rejected_without_falling_back() {
        let raw = format!("/{}", "x".repeat(400));
        let error = validate_endpoint_path(OsStr::new(&raw)).expect_err("too long");
        assert!(matches!(
            error,
            RendezvousError::InvalidExplicitEndpoint {
                reason: InvalidEndpointReason::TooLong,
                ..
            }
        ));
    }

    #[test]
    fn a_usable_override_is_returned_unchanged() {
        let path = validate_endpoint_path(OsStr::new("/tmp/corral-test.sock")).expect("usable");
        assert_eq!(path, Path::new("/tmp/corral-test.sock"));
    }
}
