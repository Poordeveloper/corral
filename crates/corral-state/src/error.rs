use std::fmt;
use std::path::PathBuf;

use corral_core::{
    Assurance, BindingId, CommandId, CorralSessionId, EvidenceSource, NodeId, ReservedNamespace,
    RunId,
};

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

/// An answer other than the one asked for, from a store that is still
/// trustworthy. Nothing was written, and the caller may carry on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// A command id already names a different semantic command. Nothing was
    /// executed and the original receipt is untouched: one command id means
    /// one immutable semantic command, for the life of the node's durable
    /// state (ADR 0002, Q12).
    CommandIdConflict {
        command: CommandId,
    },
    /// Another writer held the store for longer than the wait allows. Nothing
    /// happened, and the same call may be made again — this is the canonical
    /// transient condition, and treating it as a broken store would let one
    /// backup tool take the daemon down.
    Busy {
        detail: String,
    },
    UnknownBinding(BindingId),
    UnknownSession(CorralSessionId),
    /// The external identity is already bound to a different Session. Corral
    /// links and unlinks, never merges, so the caller is told rather than
    /// handed somebody else's binding as though the link had happened.
    BindingClaimedByAnotherSession {
        binding: BindingId,
        session: CorralSessionId,
    },
    /// Evidence that cannot assert a durable fact is not a confirmation.
    /// Writing it would be the assurance-change persistence Q15 deferred, and
    /// an append-only log with no correction event could never undo it. Not a
    /// claim that one assurance level sits below another: Corral does not
    /// order them.
    UnsupportedConfirmation {
        binding: BindingId,
        assurance: Assurance,
    },
    /// The log already holds this Run, so its start is not a fact still
    /// waiting to be appended.
    RunAlreadyRecorded(RunId),
    /// A Run brought back for appending names a Session its runtime binding
    /// does not. Distinct from an unknown Session: both may exist, and the
    /// problem is that they disagree.
    RunClaimsAnotherSession {
        run: RunId,
        claimed: CorralSessionId,
        binds: CorralSessionId,
    },
    /// Lineage that would close a loop. The log is append-only and PR2 accepts
    /// no correction event, so a cycle written once could never be removed,
    /// and every consumer walking ancestry would have to invent its own depth
    /// cap.
    LineageWouldCycle {
        child: CorralSessionId,
        parent: CorralSessionId,
    },
    /// A Session's origin is recorded once. Recording a different parent would
    /// replace a fact rather than add one.
    LineageAlreadyRecorded {
        child: CorralSessionId,
        parent: CorralSessionId,
        assurance: Assurance,
    },
    /// A Run's association is its runtime binding; no other kind of binding
    /// can carry one.
    NotARuntimeBinding(BindingId),
    /// A process episode ends once. A second end would overwrite a recorded
    /// outcome rather than add a fact.
    RunAlreadyEnded(RunId),
    /// A runtime binding names one runtime, which cannot be running two
    /// episodes at once. The earlier Run ends before another opens.
    RunAlreadyLive {
        binding: BindingId,
        run: RunId,
    },
    /// A command fingerprint is stored whole so a conflict can be read rather
    /// than guessed at, and a durable row is not a place to put unbounded
    /// client input.
    FingerprintTooLarge {
        length: usize,
        limit: usize,
    },
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
    /// A binding sits wrongly in the reserved `corral` provider namespace.
    /// The namespace records who minted an identity, and a managed runtime
    /// whose provider says otherwise — or a provider identity claiming the
    /// namespace — would leave one field meaning two things (ADR 0008 D3).
    ReservedProviderNamespace {
        binding: BindingId,
        misuse: ReservedNamespace,
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

/// Which storage-engine failures leave a usable store, and which do not.
///
/// Contention and constraint violations both end with the store exactly as it
/// was: the first never started, the second was rolled back whole. Neither is
/// a reason to stop trusting it. Everything else is: once the state layer
/// cannot explain what happened, it stops vouching rather than retrying.
impl From<rusqlite::Error> for StateError {
    fn from(error: rusqlite::Error) -> Self {
        let detail = error.to_string();
        match &error {
            rusqlite::Error::SqliteFailure(failure, _) => match failure.code {
                rusqlite::ErrorCode::ConstraintViolation => {
                    Self::Refused(Refusal::Constraint { detail })
                }
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked => {
                    Self::Refused(Refusal::Busy { detail })
                }
                _ => Self::Fatal(FatalState::Storage { detail }),
            },
            _ => Self::Fatal(FatalState::Storage { detail }),
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
            Self::Busy { detail } => {
                write!(f, "the store was held by another writer: {detail}")
            }
            Self::UnknownBinding(binding) => write!(f, "binding {binding} is not recorded"),
            Self::UnknownSession(session) => write!(f, "session {session} is not recorded"),
            Self::BindingClaimedByAnotherSession { binding, session } => write!(
                f,
                "that external identity is binding {binding}, which belongs to session {session}"
            ),
            Self::UnsupportedConfirmation { binding, assurance } => write!(
                f,
                "{assurance:?} evidence does not confirm binding {binding}"
            ),
            Self::RunAlreadyRecorded(run) => write!(f, "run {run} is already recorded"),
            Self::RunClaimsAnotherSession {
                run,
                claimed,
                binds,
            } => write!(
                f,
                "run {run} claims session {claimed}, but its runtime binding names {binds}"
            ),
            Self::LineageWouldCycle { child, parent } => write!(
                f,
                "recording session {child} as continuing {parent} would close a loop"
            ),
            Self::LineageAlreadyRecorded {
                child,
                parent,
                assurance,
            } => write!(
                f,
                "session {child} already continues {parent} on {assurance:?} evidence"
            ),
            Self::NotARuntimeBinding(binding) => {
                write!(f, "binding {binding} is not a runtime binding")
            }
            Self::RunAlreadyEnded(run) => write!(f, "run {run} has already ended"),
            Self::RunAlreadyLive { binding, run } => write!(
                f,
                "binding {binding} already has the live run {run}; one runtime is one episode"
            ),
            Self::FingerprintTooLarge { length, limit } => write!(
                f,
                "the command fingerprint is {length} bytes, and the limit is {limit}"
            ),
            Self::EvidenceCannotMintARun { binding, source } => write!(
                f,
                "binding {binding} rests on {source:?} evidence, which proves identity rather \
                 than that a runtime occurrence exists"
            ),
            Self::ControlCapableRuntimeBindingExists { session, existing } => write!(
                f,
                "session {session} already has the control-capable runtime binding {existing}"
            ),
            Self::ReservedProviderNamespace { binding, misuse } => match misuse {
                ReservedNamespace::Respected => write!(
                    f,
                    "binding {binding} respects the reserved provider namespace"
                ),
                ReservedNamespace::ManagedRuntimeWithoutIt => write!(
                    f,
                    "binding {binding} is a runtime Corral created, so its provider must be \
                     the reserved {reserved}",
                    reserved = corral_core::ProviderId::RESERVED_FOR_CORRAL
                ),
                ReservedNamespace::ClaimedByAnotherIdentity => write!(
                    f,
                    "binding {binding} is not a runtime Corral created, so it may not take the \
                     reserved provider {reserved}",
                    reserved = corral_core::ProviderId::RESERVED_FOR_CORRAL
                ),
            },
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

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
