use std::fmt;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

use corral_protocol::{PeerVersions, ProtocolError};
use corral_rendezvous::RendezvousError;

/// Where an endpoint came from when a handshake refused it.
///
/// The verdict does not change — an incompatible daemon is incompatible
/// however it got there — but the message a person reads does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivationContext {
    /// A daemon was already serving the endpoint.
    ExistingPrimary,
    /// This client started a daemon during this activation.
    ActivationAttempted,
}

/// What a client actually established about a canonical primary owner.
///
/// Kept as evidence rather than inferred from "did we spawn": an activation
/// that runs out of budget before it probes has observed nothing, and
/// reporting an owner it never saw would be inventing a runtime fact
/// (AGENTS.md §Runtime truth).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnerEvidence {
    /// No probe ran.
    NotProbed,
    /// A probe found the singleton lock held.
    Present,
    /// A probe found no owner.
    Absent,
}

/// What became of a daemon this client started.
#[derive(Clone, Copy, Debug)]
pub struct SpawnOutcome {
    pub pid: u32,
    /// `Some` when the child had already exited by the time activation gave
    /// up — the difference between "never started" and "started and refused".
    pub exit_code: Option<i32>,
}

/// Why a handshake could not establish a usable connection.
#[derive(Debug)]
pub enum HandshakeFault {
    /// The peer's hello is missing required identity fields, or ill-typed.
    Malformed { detail: String },
    /// A legal frame arrived where the bootstrap does not allow it.
    ProtocolViolation { detail: String },
    /// The daemon refused the hello with a typed error.
    Refused(ProtocolError),
    /// Both sides evaluate the same symmetric predicate, so disagreeing is an
    /// internal protocol bug rather than an ordinary incompatibility.
    DivergentCompatibilityVerdict {
        ours: PeerVersions,
        theirs: PeerVersions,
    },
}

/// Why a surface has no usable daemon connection.
///
/// The layers are kept apart on purpose — resolution and configuration, then
/// activation, then transport reachability, then handshake — because callers
/// act differently on each and a single "activation failed" bucket would hide
/// which one happened.
#[derive(Debug)]
pub enum ActivationError {
    /// Resolution or configuration: no endpoint may be attempted at all.
    Rendezvous(RendezvousError),
    /// An explicit endpoint could not be reached. Terminal by design: an
    /// override redirects this client, it never licenses starting a second
    /// primary daemon somewhere else.
    ExplicitEndpointUnavailable {
        endpoint: PathBuf,
        source: io::Error,
    },
    /// The canonical endpoint failed to connect for a reason activation
    /// cannot repair, such as a permission fault.
    Endpoint {
        endpoint: PathBuf,
        source: io::Error,
    },
    /// The overall budget ran out before the rendezvous could be assessed.
    /// Distinct from `OwnerPresentButUnreachable`, which requires having seen
    /// an owner.
    ActivationBudgetExpired {
        endpoint: PathBuf,
        deadline: Duration,
        owner: OwnerEvidence,
    },
    /// A primary daemon holds the lock and its endpoint never became usable.
    OwnerPresentButUnreachable {
        lock_path: PathBuf,
        endpoint: PathBuf,
        deadline: Duration,
    },
    /// This client was allowed to start a daemon and no usable one appeared.
    SpawnedDaemonDidNotBecomeReady {
        endpoint: PathBuf,
        deadline: Duration,
        spawn_result: SpawnOutcome,
    },
    /// `corrald` is not installed beside the running executable.
    InstallIntegrity { expected: PathBuf, detail: String },
    /// The daemon binary exists and could not be started.
    Spawn { program: PathBuf, source: io::Error },
    /// A reachable daemon this build cannot talk to. Terminal immediately: no
    /// retry, no fallback, no second daemon, and no authority to kill this one.
    IncompatibleDaemon {
        ours: PeerVersions,
        theirs: PeerVersions,
        endpoint: PathBuf,
        context: ActivationContext,
    },
    /// The bootstrap itself failed.
    Handshake {
        endpoint: PathBuf,
        fault: HandshakeFault,
    },
}

/// Why a request on an established connection failed.
#[derive(Debug)]
pub enum RequestError {
    /// The daemon went away. Never replayed automatically: replay needs
    /// idempotency semantics only the caller can supply.
    DaemonConnectionLost { endpoint: PathBuf },
    /// The daemon answered with a typed refusal.
    Refused(ProtocolError),
    /// The daemon answered with something this protocol does not permit.
    Protocol { detail: String },
}

impl From<RendezvousError> for ActivationError {
    fn from(error: RendezvousError) -> Self {
        Self::Rendezvous(error)
    }
}

impl fmt::Display for ActivationContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::ExistingPrimary => "already running",
            Self::ActivationAttempted => "started by this command",
        };
        f.write_str(text)
    }
}

impl fmt::Display for OwnerEvidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::NotProbed => {
                "the budget ran out before this command could tell whether a corrald is running"
            }
            Self::Present => "a corrald holds the singleton lock",
            Self::Absent => "no corrald held the singleton lock",
        };
        f.write_str(text)
    }
}

impl fmt::Display for HandshakeFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed { detail } => write!(f, "the daemon's hello was malformed: {detail}"),
            Self::ProtocolViolation { detail } => {
                write!(f, "the daemon broke the bootstrap contract: {detail}")
            }
            Self::Refused(error) => write!(f, "the daemon refused the hello: {error}"),
            Self::DivergentCompatibilityVerdict { ours, theirs } => write!(
                f,
                "the two sides disagreed about compatibility (this build speaks {} and needs \
                 at least {}; the daemon speaks {} and needs at least {}) — that is an internal \
                 protocol bug, so the connection was failed rather than used",
                ours.protocol_version,
                ours.min_compatible_peer_version,
                theirs.protocol_version,
                theirs.min_compatible_peer_version,
            ),
        }
    }
}

impl fmt::Display for ActivationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rendezvous(error) => write!(f, "{error}"),
            Self::ExplicitEndpointUnavailable { endpoint, source } => write!(
                f,
                "CORRAL_ENDPOINT points at {} and it did not become usable: {source}. An \
                 endpoint override redirects this command; it never starts a daemon of its own",
                endpoint.display()
            ),
            Self::Endpoint { endpoint, source } => {
                write!(f, "{} could not be reached: {source}", endpoint.display())
            }
            Self::ActivationBudgetExpired {
                endpoint,
                deadline,
                owner,
            } => write!(
                f,
                "{} did not become usable within {:.0?}, and {owner}",
                endpoint.display(),
                deadline
            ),
            Self::OwnerPresentButUnreachable {
                lock_path,
                endpoint,
                deadline,
            } => write!(
                f,
                "a corrald holds {} but {} did not become usable within {:.0?}. Nothing else may \
                 start a daemon while that lock is held; it is released when that corrald exits",
                lock_path.display(),
                endpoint.display(),
                deadline
            ),
            Self::SpawnedDaemonDidNotBecomeReady {
                endpoint,
                deadline,
                spawn_result,
            } => match spawn_result.exit_code {
                Some(code) => write!(
                    f,
                    "corrald was started (pid {}) and exited with status {code} before {} became \
                     usable within {:.0?}",
                    spawn_result.pid,
                    endpoint.display(),
                    deadline
                ),
                None => write!(
                    f,
                    "corrald was started (pid {}) and {} did not become usable within {:.0?}",
                    spawn_result.pid,
                    endpoint.display(),
                    deadline
                ),
            },
            Self::InstallIntegrity { expected, detail } => write!(
                f,
                "corrald was not found beside corral at {} ({detail}); reinstall or repair the \
                 installation",
                expected.display()
            ),
            Self::Spawn { program, source } => {
                write!(f, "{} could not be started: {source}", program.display())
            }
            Self::IncompatibleDaemon {
                ours,
                theirs,
                endpoint,
                context,
            } => write!(
                f,
                "this build speaks protocol {} and needs a daemon of at least {}; the corrald at \
                 {} ({context}) speaks protocol {} and needs a client of at least {}",
                ours.protocol_version,
                ours.min_compatible_peer_version,
                endpoint.display(),
                theirs.protocol_version,
                theirs.min_compatible_peer_version,
            ),
            Self::Handshake { endpoint, fault } => {
                write!(
                    f,
                    "the handshake with {} failed: {fault}",
                    endpoint.display()
                )
            }
        }
    }
}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DaemonConnectionLost { endpoint } => write!(
                f,
                "the corrald at {} closed the connection before answering; the request was not \
                 retried",
                endpoint.display()
            ),
            Self::Refused(error) => write!(f, "the daemon refused the request: {error}"),
            Self::Protocol { detail } => write!(f, "the daemon broke the protocol: {detail}"),
        }
    }
}

impl std::error::Error for ActivationError {}
impl std::error::Error for RequestError {}
