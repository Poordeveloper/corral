use std::ffi::OsStr;
use std::fs::DirBuilder;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use uzers::os::unix::UserExt;

use crate::error::{InvalidEndpointReason, RendezvousError};

const CORRAL_DIR: &str = ".corral";
const RUN_DIR: &str = "run";
const LOG_DIR: &str = "log";
const STATE_DIR: &str = "state";
/// Diagnostic evidence beside — never inside — durable state: deletable,
/// unrebuildable, and never read back into product truth (ADR 0015 D8).
const DIAGNOSTICS_DIR: &str = "diagnostics";
const SOCKET_FILE: &str = "corrald.sock";
/// The hook endpoint's pathname.
///
/// Shorter than the canonical socket's, deliberately: the canonical one is
/// already length-checked at construction, so a name no longer than it cannot
/// be the one that overflows `sun_path` on a root the canonical socket fits in.
const HOOK_SOCKET_FILE: &str = "hook.sock";
const LOCK_FILE: &str = "corrald.lock";
const LOG_FILE: &str = "corrald.log";
const REGISTRY_FILE: &str = "registry.sqlite3";
/// Where per-launch provider configuration Corral owns is written.
///
/// Under the durable-state tree rather than the runtime one because a launch
/// outlives neither a daemon nor a reboot cleanly: these files are cleaned on
/// ownership evidence, never by being found in a directory somebody sweeps
/// (ADR 0004 D6; PR5 plan design 8).
const LAUNCH_DIR: &str = "launch";

/// Runtime and log directories are user-private: the flock and these modes are
/// a transport fence, deliberately not a security boundary (ADR 0001).
const PRIVATE_DIR_MODE: u32 = 0o700;

/// The canonical rendezvous of one OS account.
///
/// Derivation is a pure function of the Corral root, and in production that
/// root is a pure function of the OS-account home. So every process of the
/// same effective OS user on the same host computes the same paths whatever
/// their shell, session type, cron or ssh environment looks like (ADR 0001 D1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RendezvousPaths {
    root: PathBuf,
    run_dir: PathBuf,
    socket: PathBuf,
    hook_socket: PathBuf,
    lock: PathBuf,
    log_dir: PathBuf,
    log_file: PathBuf,
    state_dir: PathBuf,
    diagnostics_dir: PathBuf,
    registry: PathBuf,
    launch_dir: PathBuf,
}

impl RendezvousPaths {
    /// The canonical rendezvous this process must use.
    ///
    /// Client and daemon both arrive here, so neither can hold a different
    /// idea of which daemon is the account's primary.
    pub fn canonical() -> Result<Self, RendezvousError> {
        Self::for_corral_root(corral_root()?)
    }

    /// The layout rule, given a Corral root.
    pub fn for_corral_root(root: impl AsRef<Path>) -> Result<Self, RendezvousError> {
        let root = root.as_ref();
        let run_dir = root.join(RUN_DIR);
        let log_dir = root.join(LOG_DIR);
        let state_dir = root.join(STATE_DIR);
        let diagnostics_dir = root.join(DIAGNOSTICS_DIR);
        let socket = run_dir.join(SOCKET_FILE);

        if socket_address_length_exceeded(&socket) {
            return Err(RendezvousError::CanonicalEndpointTooLong { path: socket });
        }

        Ok(Self {
            lock: run_dir.join(LOCK_FILE),
            hook_socket: run_dir.join(HOOK_SOCKET_FILE),
            log_file: log_dir.join(LOG_FILE),
            registry: state_dir.join(REGISTRY_FILE),
            launch_dir: state_dir.join(LAUNCH_DIR),
            root: root.to_path_buf(),
            socket,
            run_dir,
            log_dir,
            state_dir,
            diagnostics_dir,
        })
    }

    /// The production root of an OS account, and the layout under it.
    pub fn for_account_home(home: impl AsRef<Path>) -> Result<Self, RendezvousError> {
        Self::for_corral_root(home.as_ref().join(CORRAL_DIR))
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// Where provider hook events reach this account's daemon.
    ///
    /// A second local socket beside the canonical one, created and removed by
    /// the daemon alone. Separate rather than multiplexed so that
    /// evidence-only is a structural fact: no session method and no control
    /// surface is reachable from it (ADR 0004 D2).
    ///
    /// Not a singleton artifact. The canonical socket is the meeting place the
    /// claim protects; this one exists for as long as the daemon holding that
    /// claim is serving, and nothing probes it to decide whether a daemon
    /// exists.
    pub fn hook_socket(&self) -> &Path {
        &self.hook_socket
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

    /// The registry store of this account's daemon.
    ///
    /// Not a rendezvous artifact: it carries no singleton rules and nobody
    /// cleans it up as stale. It lives here because the Corral root's layout
    /// has one owner, and a second crate deriving paths under the root would
    /// be a second answer to where an account's files are.
    pub fn registry(&self) -> &Path {
        &self.registry
    }

    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    /// The account home the root sits under — where the providers keep their
    /// own session stores, which history enumeration reads by name.
    pub fn account_home(&self) -> &Path {
        self.root.parent().unwrap_or(&self.root)
    }

    /// Where diagnostic journals live: the attention journal and whatever
    /// later diagnostics need a home that is plainly not product truth.
    pub fn diagnostics_dir(&self) -> &Path {
        &self.diagnostics_dir
    }

    /// The directory holding the per-launch provider configuration Corral
    /// writes for a managed session.
    pub fn launch_dir(&self) -> &Path {
        &self.launch_dir
    }

    /// Create the user-private runtime directory. Idempotent.
    pub fn ensure_run_dir(&self) -> Result<(), RendezvousError> {
        self.ensure_private_tree(&self.run_dir)
    }

    /// Create the user-private log directory. Idempotent.
    pub fn ensure_log_dir(&self) -> Result<(), RendezvousError> {
        self.ensure_private_tree(&self.log_dir)
    }

    /// Create the user-private durable-state directory. Idempotent.
    pub fn ensure_state_dir(&self) -> Result<(), RendezvousError> {
        self.ensure_private_tree(&self.state_dir)
    }

    /// Create the user-private per-launch configuration directory. Idempotent.
    pub fn ensure_launch_dir(&self) -> Result<(), RendezvousError> {
        self.ensure_private_tree(&self.state_dir)?;
        ensure_private_dir(&self.launch_dir)
    }

    /// Check every directory Corral owns on the way down, not just the leaf.
    ///
    /// A private `run` inside a world-writable root is not private: another
    /// account can replace `run` with a symlink to somewhere it controls, and
    /// the leaf check would happily report `0700` about the target.
    fn ensure_private_tree(&self, leaf: &Path) -> Result<(), RendezvousError> {
        ensure_private_dir(&self.root)?;
        ensure_private_dir(leaf)
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

/// Create a user-private directory, or confirm the existing one is private.
///
/// `mode` applies only to components this call creates, so an existing
/// directory has to be checked rather than assumed: S6(d) makes user-private
/// runtime and log directories PR1 correctness, and a directory left open to
/// the group would silently widen the transport fence.
///
/// A symlinked directory is followed deliberately. The rule the grill froze is
/// that *lock and socket creation* must not follow an unexpected substitution
/// — enforced at those final components with `O_NOFOLLOW` and
/// `symlink_metadata` — not that a person may never point `~/.corral` at
/// another volume.
fn ensure_private_dir(path: &Path) -> Result<(), RendezvousError> {
    DirBuilder::new()
        .recursive(true)
        .mode(PRIVATE_DIR_MODE)
        .create(path)
        .map_err(|source| RendezvousError::RuntimeDirectory {
            path: path.to_path_buf(),
            source,
        })?;

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

    let owner = metadata.uid();
    let us = uzers::get_effective_uid();
    if owner != us {
        return Err(RendezvousError::RuntimeDirectoryNotOwned {
            path: path.to_path_buf(),
            owner,
            expected: us,
        });
    }

    // Exactly `0700`, not merely "no group or other bits": a directory the
    // owner cannot search fails later with a bare EACCES from whatever
    // touches it first, which is a worse answer than naming the mode here.
    // The set-user/group and sticky bits are left out of the comparison
    // because they say nothing about who can read this directory, and on BSD
    // filesystems a child can inherit set-group from its parent.
    let mode = metadata.permissions().mode() & 0o777;
    if mode != PRIVATE_DIR_MODE {
        // Fail closed and name the fix rather than quietly re-tightening a
        // directory this run did not create.
        return Err(RendezvousError::RuntimeDirectoryNotPrivate {
            path: path.to_path_buf(),
            mode,
        });
    }
    Ok(())
}

/// The Corral root this process must use.
///
/// In production this is `<account home>/.corral` and nothing can move it. A
/// `test-support` build resolves a test namespace instead when one is set — a
/// substitution, never a fallback: production resolution failing does not
/// reach for the namespace, and the namespace failing does not reach for
/// production.
fn corral_root() -> Result<PathBuf, RendezvousError> {
    if let Some(root) = test_namespace::root()? {
        return Ok(root);
    }
    Ok(account_home()?.join(CORRAL_DIR))
}

/// The home directory whose provider dotfiles Corral's integration reads and
/// writes — `~/.claude/settings.json`, `~/.codex/config.toml` (ADR 0013).
///
/// Resolved through the account database for the same reason the rendezvous
/// is (ADR 0001 D1): a shell variable must not be able to point Corral's one
/// mutator at a different account's configuration. It follows the test
/// namespace for a blunter reason — a test that resolved the real home would
/// write into the developer's own provider configuration.
pub fn provider_home() -> Result<PathBuf, RendezvousError> {
    if let Some(root) = test_namespace::root()? {
        return Ok(root.join(TEST_PROVIDER_HOME));
    }
    account_home()
}

/// Where provider dotfiles live under a substituted namespace. Inside the
/// test root, so one scratch directory still holds everything a test creates.
const TEST_PROVIDER_HOME: &str = "provider-home";

/// The home directory of the effective OS user, from the account database.
///
/// Never `$HOME`: the canonical rendezvous is user-wide, so a shell variable
/// must not be able to give one OS account two primary daemons (ADR 0001 D1).
/// The uid is the effective one, matching the filesystem and process authority
/// Corral actually acts with.
fn account_home() -> Result<PathBuf, RendezvousError> {
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

/// The test-only rendezvous namespace seam (ADR 0001, "Test injection").
///
/// Not a runtime-policy knob and not a configuration surface: it names a whole
/// alternative Corral root, so a process-level test can exercise real
/// resolution, locking, socket binding and sibling auto-spawn without writing
/// into the developer's own account.
///
/// Normal production binaries do not recognize the variable at all; only
/// explicit `test-support` builds do, and `test-support` is not a default
/// feature. Everything downstream of the root — path-length limits, private
/// directory creation, lock and socket artifact rules — is the same code a
/// production root runs through.
mod test_namespace {
    use std::path::PathBuf;

    use crate::error::RendezvousError;

    #[cfg(feature = "test-support")]
    const TEST_ROOT: &str = "CORRAL_TEST_ROOT";

    #[cfg(feature = "test-support")]
    pub(super) fn root() -> Result<Option<PathBuf>, RendezvousError> {
        let Some(raw) = std::env::var_os(TEST_ROOT) else {
            return Ok(None);
        };
        let path = PathBuf::from(&raw);
        if path.as_os_str().is_empty() {
            return Err(RendezvousError::InvalidTestNamespace {
                raw,
                detail: "it is empty",
            });
        }
        if !path.is_absolute() {
            // Quietly using the production root here would turn the seam into
            // a conditional configuration surface, which is the one thing it
            // may never become.
            return Err(RendezvousError::InvalidTestNamespace {
                raw,
                detail: "it is not an absolute path",
            });
        }
        Ok(Some(path))
    }

    #[cfg(not(feature = "test-support"))]
    pub(super) fn root() -> Result<Option<PathBuf>, RendezvousError> {
        Ok(None)
    }
}

#[cfg(test)]
#[path = "paths_tests.rs"]
mod tests;
