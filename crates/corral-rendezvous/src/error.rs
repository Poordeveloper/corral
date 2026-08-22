use std::ffi::OsString;
use std::fmt;
use std::io;
use std::path::PathBuf;

/// Everything that can go wrong before a client or daemon has an endpoint it
/// may legitimately use.
///
/// The variants are the resolution/configuration and filesystem layers of the
/// ADR 0001 failure table. They are deliberately distinct facts: a permission
/// error is never reported as "a daemon exists", and a corrupt artifact is
/// never repaired by deleting it.
#[derive(Debug)]
pub enum RendezvousError {
    /// The effective OS user has no usable home directory in the account
    /// database, so the account has no canonical rendezvous at all.
    AccountHomeUnresolvable { uid: u32, detail: String },
    /// The canonical socket path cannot address a Unix socket on this
    /// platform. Externally managed installs can still be reached with an
    /// explicit endpoint.
    CanonicalEndpointTooLong { path: PathBuf },
    /// An explicit endpoint override is not a usable endpoint at all.
    InvalidExplicitEndpoint {
        raw: OsString,
        reason: InvalidEndpointReason,
    },
    /// A user-private runtime directory could not be created or is not a
    /// directory.
    RuntimeDirectory { path: PathBuf, source: io::Error },
    /// The singleton lock could not be opened or locked for a reason other
    /// than contention. Never evidence that a daemon exists.
    Lock { path: PathBuf, source: io::Error },
    /// Something that is not a Unix socket occupies the socket pathname.
    /// Stale-socket cleanup is not a file-deletion primitive, so this fails
    /// closed and deletes nothing.
    OccupiedSocketPath { path: PathBuf, found: FileKind },
    /// Inspecting or removing the socket pathname failed.
    SocketPathname { path: PathBuf, source: io::Error },
    /// Only reachable in a `test-support` build: the test rendezvous namespace
    /// was set to something that cannot be a Corral root. Production binaries
    /// do not recognize the seam, so they cannot produce this.
    #[cfg(feature = "test-support")]
    InvalidTestNamespace { raw: OsString, detail: &'static str },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvalidEndpointReason {
    Empty,
    Relative,
    TooLong,
}

/// What was found where a Unix socket was expected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Directory,
    RegularFile,
    Symlink,
    Other,
}

impl fmt::Display for InvalidEndpointReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Empty => "it is empty",
            Self::Relative => "it is not an absolute path",
            Self::TooLong => "it is longer than a Unix socket address allows",
        };
        f.write_str(text)
    }
}

impl fmt::Display for FileKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Directory => "a directory",
            Self::RegularFile => "a regular file",
            Self::Symlink => "a symbolic link",
            Self::Other => "an unexpected filesystem object",
        };
        f.write_str(text)
    }
}

impl fmt::Display for RendezvousError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AccountHomeUnresolvable { uid, detail } => write!(
                f,
                "the OS account database has no usable home directory for uid {uid}: {detail}"
            ),
            Self::CanonicalEndpointTooLong { path } => write!(
                f,
                "the canonical endpoint {} is longer than a Unix socket address allows",
                path.display()
            ),
            Self::InvalidExplicitEndpoint { raw, reason } => write!(
                f,
                "CORRAL_ENDPOINT {:?} is not a usable endpoint: {reason}",
                raw
            ),
            Self::RuntimeDirectory { path, source } => write!(
                f,
                "the runtime directory {} is unusable: {source}",
                path.display()
            ),
            Self::Lock { path, source } => write!(
                f,
                "the singleton lock {} could not be used: {source}",
                path.display()
            ),
            Self::OccupiedSocketPath { path, found } => write!(
                f,
                "{} is {found}, not a Corral socket; nothing was removed",
                path.display()
            ),
            Self::SocketPathname { path, source } => {
                write!(f, "the socket pathname {} : {source}", path.display())
            }
            #[cfg(feature = "test-support")]
            Self::InvalidTestNamespace { raw, detail } => write!(
                f,
                "the test rendezvous namespace {raw:?} is not a usable Corral root: {detail}"
            ),
        }
    }
}

impl std::error::Error for RendezvousError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::RuntimeDirectory { source, .. }
            | Self::Lock { source, .. }
            | Self::SocketPathname { source, .. } => Some(source),
            _ => None,
        }
    }
}
