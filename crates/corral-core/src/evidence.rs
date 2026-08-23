use std::time::SystemTime;

use crate::assurance::Assurance;

/// Where a fact came from.
///
/// Ranked for turn state in `ARCHITECTURE.md` §2, but this type does not rank:
/// authority is qualified by freshness, and the phase that derives status owns
/// that computation (PR8). What the domain fixes here is which classes may
/// mint a Run, because that boundary is architecture law rather than tuning.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EvidenceSource {
    /// Corral created the thing itself and owns the fact by construction.
    CorralConstructed,
    /// The node's accepted runtime-observation mechanism saw a concrete
    /// runtime. For host-native M1 that is typically process identity plus
    /// OS-level liveness evidence, but the class is named by its authority,
    /// not by a pid: a runtime owner with a stronger handle must not have to
    /// impersonate one (ADR 0002 D2).
    NodeRuntimeObservation,
    /// A provider-native hook or event.
    ProviderHook,
    /// A signal carried in the runtime's own output stream.
    InBandSignal,
    /// Terminal or screen detection.
    ScreenDetection,
    /// Provider history or transcript records.
    HistoryRecord,
    /// cwd / time / process correlation.
    Correlation,
    /// The user said so.
    UserAssertion,
}

impl EvidenceSource {
    /// Whether this class of evidence may mint a Run (ADR 0002 D2).
    ///
    /// Semantic evidence proves identity, never live runtime truth: a hook
    /// having fired does not mean the runtime is alive now (AGENTS.md
    /// §Runtime truth).
    #[must_use]
    pub fn establishes_runtime_occurrence(self) -> bool {
        match self {
            Self::CorralConstructed | Self::NodeRuntimeObservation => true,
            Self::ProviderHook
            | Self::InBandSignal
            | Self::ScreenDetection
            | Self::HistoryRecord
            | Self::Correlation
            | Self::UserAssertion => false,
        }
    }
}

/// One observation, with everything needed to judge what it may claim.
///
/// Freshness is `observed_at` against the reader's clock; this type records
/// the observation and does not decide whether it has gone stale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Evidence {
    source: EvidenceSource,
    assurance: Assurance,
    observed_at: SystemTime,
}

impl Evidence {
    #[must_use]
    pub fn new(source: EvidenceSource, assurance: Assurance, observed_at: SystemTime) -> Self {
        Self {
            source,
            assurance,
            observed_at,
        }
    }

    #[must_use]
    pub fn source(&self) -> EvidenceSource {
        self.source
    }

    #[must_use]
    pub fn assurance(&self) -> Assurance {
        self.assurance
    }

    #[must_use]
    pub fn observed_at(&self) -> SystemTime {
        self.observed_at
    }
}

#[cfg(test)]
#[path = "evidence_tests.rs"]
mod tests;
