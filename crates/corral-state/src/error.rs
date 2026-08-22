use std::fmt;
use std::path::PathBuf;

use corral_core::{BindingId, CommandId, CorralSessionId, EvidenceSource, NodeId, RunId};

/// Everything the registry store can answer with other than the fact asked
/// for.
///
/// The split is what a caller has to act on. A refusal leaves the store intact
/// and trustworthy — the write did not happen and the caller may carry on. A
/// fatal state means the store can no longer vouch for durable truth, and the
/// only correct response is to stop serving state-derived claims (ADR 0002,
/// Q14).
#[derive(Clone, Debug)]
pub enum StateError {
    Refused(Refusal),
    Fatal(FatalState),
}

/// A write the store declined. The store is still usable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// A command id already names a different semantic command. Nothing was
    /// executed and the original receipt is untouched: one command id means
    /// one immutable semantic command, for the life of the node's durable
    /// state (ADR 0002, Q12).
    CommandIdConflict {
        command: CommandId,
    },
    UnknownBinding(BindingId),
    /// A Run's association is its runtime binding; no other kind of binding
    /// can carry one.
    NotARuntimeBinding(BindingId),
    /// A process episode ends once. A second end would overwrite a recorded
    /// outcome rather than add a fact.
    RunAlreadyEnded(RunId),
    /// A Run may be minted only from independent authoritative evidence that a
    /// concrete runtime occurrence exists. Semantic evidence proves identity,
    /// never live runtime truth (ADR 0002 D2).
    EvidenceCannotMintARun {
        binding: BindingId,
        source: EvidenceSource,
    },
    /// At most one control-capable runtime binding is active per Session
    /// (`ARCHITECTURE.md` §1). Supersession has no producer and no accepted
    /// event yet, so the second acquisition fails closed rather than quietly
    /// displacing the first.
    ControlCapableRuntimeBindingExists {
        session: CorralSessionId,
        existing: BindingId,
    },
    /// The store's own integrity rules rejected the write. The transaction
    /// rolled back whole, so neither the event nor the projection landed.
    Constraint {
        detail: String,
    },
}

/// The store cannot vouch for durable truth any more.
///
/// Every variant is a conclusion, not a transient hiccup: reaching one means
/// `corrald` must fail closed rather than return a normal-looking projection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FatalState {
    /// The database could not be opened, created, or integrity-checked.
    Unopenable { path: PathBuf, detail: String },
    /// The store on disk is not the schema this build knows. No migration
    /// exists, so a mismatch is never guessed at.
    SchemaVersionMismatch { expected: u32, found: Option<u32> },
    /// The store is no longer the one this process validated at startup —
    /// replaced or rewritten underneath it. Every fact read since is suspect.
    StoreIdentityChanged {
        expected: NodeId,
        found: Option<String>,
    },
    /// A storage-engine failure that is not a constraint violation.
    Storage { detail: String },
    /// A recorded fact this build cannot interpret. Projections derived from
    /// a log with an unreadable event would be silently incomplete, so this is
    /// fatal rather than skipped.
    Unreadable { detail: String },
    /// A timestamp outside what the store can represent. A clock this far off
    /// makes recorded occurrence times meaningless.
    UnrepresentableTime,
}

impl StateError {
    #[must_use]
    pub fn is_fatal(&self) -> bool {
        matches!(self, Self::Fatal(_))
    }
}

impl From<Refusal> for StateError {
    fn from(refusal: Refusal) -> Self {
        Self::Refused(refusal)
    }
}

impl From<FatalState> for StateError {
    fn from(fatal: FatalState) -> Self {
        Self::Fatal(fatal)
    }
}

/// Constraint violations are the storage engine refusing a write it has fully
/// rolled back; everything else it reports is a failure to store at all.
///
/// The mapping is deliberately strict in the second case: once the state layer
/// cannot explain what happened, it stops vouching rather than retrying.
/// Narrowing which engine failures are genuinely transient is a follow-up for
/// the phase that has a producer under real load.
impl From<rusqlite::Error> for StateError {
    fn from(error: rusqlite::Error) -> Self {
        match &error {
            rusqlite::Error::SqliteFailure(failure, _)
                if failure.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Self::Refused(Refusal::Constraint {
                    detail: error.to_string(),
                })
            }
            _ => Self::Fatal(FatalState::Storage {
                detail: error.to_string(),
            }),
        }
    }
}

impl fmt::Display for StateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(refusal) => write!(f, "{refusal}"),
            Self::Fatal(fatal) => write!(f, "{fatal}"),
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CommandIdConflict { command } => write!(
                f,
                "command id {} already names a different command; nothing was executed",
                command.as_str()
            ),
            Self::UnknownBinding(binding) => write!(f, "binding {binding} is not recorded"),
            Self::NotARuntimeBinding(binding) => {
                write!(f, "binding {binding} is not a runtime binding")
            }
            Self::RunAlreadyEnded(run) => write!(f, "run {run} has already ended"),
            Self::EvidenceCannotMintARun { binding, source } => write!(
                f,
                "binding {binding} rests on {source:?} evidence, which proves identity rather \
                 than that a runtime occurrence exists"
            ),
            Self::ControlCapableRuntimeBindingExists { session, existing } => write!(
                f,
                "session {session} already has the control-capable runtime binding {existing}"
            ),
            Self::Constraint { detail } => write!(f, "the store refused the write: {detail}"),
        }
    }
}

impl fmt::Display for FatalState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unopenable { path, detail } => {
                write!(
                    f,
                    "the registry store {} is unusable: {detail}",
                    path.display()
                )
            }
            Self::SchemaVersionMismatch { expected, found } => match found {
                Some(found) => write!(
                    f,
                    "the registry store is schema {found}; this build knows schema {expected}"
                ),
                None => write!(
                    f,
                    "the registry store states no schema version; this build knows schema {expected}"
                ),
            },
            Self::StoreIdentityChanged { expected, found } => match found {
                Some(found) => write!(
                    f,
                    "the registry store now identifies as node {found}, not {expected}"
                ),
                None => write!(
                    f,
                    "the registry store no longer identifies as node {expected}"
                ),
            },
            Self::Storage { detail } => write!(f, "the registry store failed: {detail}"),
            Self::Unreadable { detail } => {
                write!(
                    f,
                    "the registry store holds a fact this build cannot read: {detail}"
                )
            }
            Self::UnrepresentableTime => {
                f.write_str("a timestamp is outside what the registry store can represent")
            }
        }
    }
}

impl std::error::Error for StateError {}
