use std::fmt;
use std::time::SystemTime;

use crate::assurance::Assurance;
use crate::id::CorralSessionId;

/// The logical unit of AI work: Corral's primary object.
///
/// Identity never depends on any process that ran it, so nothing about a Run —
/// its pid, terminal, start time, or provider-side id — appears here.
///
/// Archived and Deleted are lifecycle axes in `ARCHITECTURE.md` §1, and are
/// deliberately absent: no accepted durable event expresses either, and a
/// projection field no event can justify would let the store know facts the
/// log does not (ADR 0002 D6, projection law).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Session {
    id: CorralSessionId,
    created_at: SystemTime,
}

impl Session {
    #[must_use]
    pub fn new(id: CorralSessionId, created_at: SystemTime) -> Self {
        Self { id, created_at }
    }

    #[must_use]
    pub fn id(&self) -> CorralSessionId {
        self.id
    }

    #[must_use]
    pub fn created_at(&self) -> SystemTime {
        self.created_at
    }
}

/// A Corral-owned edge from a Session to the one it continued from.
///
/// Handing context into a fresh provider session produces a new Session with
/// an edge to its predecessor — never a new Run of the same Session, which
/// would put two live agents behind one row (ADR 0002 D4). The edge is a fact
/// between two Sessions, not a binding: bindings relate a Session to an
/// external identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionLineage {
    child: CorralSessionId,
    parent: CorralSessionId,
    assurance: Assurance,
}

impl SessionLineage {
    /// Record that `child` continued `parent`.
    ///
    /// Heuristic similarity may suggest lineage, but must not create this fact
    /// (ADR 0002 D4): an externally observed fork names no parent, and a
    /// guessed edge would be inherited by every control decision downstream.
    /// Refusing here rather than at the store means no producer can reach a
    /// durable write with an edge the domain would not have allowed.
    pub fn record(
        child: CorralSessionId,
        parent: CorralSessionId,
        assurance: Assurance,
    ) -> Result<Self, LineageRefused> {
        if child == parent {
            return Err(LineageRefused::SelfParent);
        }
        if !assurance.may_assert_durable_fact() {
            return Err(LineageRefused::UnsupportedAssurance(assurance));
        }
        Ok(Self {
            child,
            parent,
            assurance,
        })
    }

    #[must_use]
    pub fn child(&self) -> CorralSessionId {
        self.child
    }

    #[must_use]
    pub fn parent(&self) -> CorralSessionId {
        self.parent
    }

    #[must_use]
    pub fn assurance(&self) -> Assurance {
        self.assurance
    }
}

/// Why an edge was not recorded. Recording nothing is the correct outcome, not
/// a degraded one: unlinked is not the same as unrelated (ADR 0002 D7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineageRefused {
    UnsupportedAssurance(Assurance),
    SelfParent,
}

impl fmt::Display for LineageRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedAssurance(assurance) => write!(
                f,
                "{assurance:?} evidence may suggest lineage but may not record it"
            ),
            Self::SelfParent => f.write_str("a Session cannot continue itself"),
        }
    }
}

impl std::error::Error for LineageRefused {}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
