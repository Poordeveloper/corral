use std::fmt;
use std::time::SystemTime;

use crate::assurance::Assurance;
use crate::evidence::Evidence;
use crate::external_name::{ExternalId, ProviderId};
use crate::id::{BindingId, CorralSessionId, NodeId};

/// What kind of external identity a binding points at.
///
/// Named `RuntimeBinding`, `ProviderSessionBinding` and so on in
/// `ARCHITECTURE.md` §1; the suffix is the `Binding` type itself here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BindingKind {
    ProviderSession,
    Runtime,
    Terminal,
    History,
}

/// How a binding came to exist.
///
/// Distinct from the evidence supporting it: evidence is re-evaluated as
/// observations change, provenance is the historical fact of who created the
/// edge and never moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Provenance {
    /// Corral created the thing the binding points at.
    CorralCreated,
    /// Corral found it already running or already recorded.
    Discovered,
    /// The user linked it.
    UserLinked,
}

/// The uniqueness key that makes discovery idempotent.
///
/// Re-scanning, re-watching, or restarting resolves a previously seen external
/// identity to its existing Session through this key — never to a duplicate
/// Session (`ARCHITECTURE.md` §1). The Session is deliberately not part of the
/// key: the key is what Corral looks a Session *up* by.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BindingKey {
    node: NodeId,
    kind: BindingKind,
    provider: ProviderId,
    external_id: ExternalId,
}

impl BindingKey {
    #[must_use]
    pub fn new(
        node: NodeId,
        kind: BindingKind,
        provider: ProviderId,
        external_id: ExternalId,
    ) -> Self {
        Self {
            node,
            kind,
            provider,
            external_id,
        }
    }

    #[must_use]
    pub fn node(&self) -> NodeId {
        self.node
    }

    #[must_use]
    pub fn kind(&self) -> BindingKind {
        self.kind
    }

    #[must_use]
    pub fn provider(&self) -> &ProviderId {
        &self.provider
    }

    #[must_use]
    pub fn external_id(&self) -> &ExternalId {
        &self.external_id
    }
}

/// An edge from a Session to one external identity.
///
/// The binding is where association assurance lives, and the only place
/// control eligibility can be resolved from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    id: BindingId,
    session: CorralSessionId,
    key: BindingKey,
    provenance: Provenance,
    evidence: Evidence,
    created_at: SystemTime,
}

impl Binding {
    #[must_use]
    pub fn new(
        id: BindingId,
        session: CorralSessionId,
        key: BindingKey,
        provenance: Provenance,
        evidence: Evidence,
        created_at: SystemTime,
    ) -> Self {
        Self {
            id,
            session,
            key,
            provenance,
            evidence,
            created_at,
        }
    }

    #[must_use]
    pub fn id(&self) -> BindingId {
        self.id
    }

    #[must_use]
    pub fn session(&self) -> CorralSessionId {
        self.session
    }

    #[must_use]
    pub fn key(&self) -> &BindingKey {
        &self.key
    }

    #[must_use]
    pub fn kind(&self) -> BindingKind {
        self.key.kind()
    }

    #[must_use]
    pub fn provenance(&self) -> Provenance {
        self.provenance
    }

    #[must_use]
    pub fn evidence(&self) -> Evidence {
        self.evidence
    }

    #[must_use]
    pub fn assurance(&self) -> Assurance {
        self.evidence.assurance()
    }

    #[must_use]
    pub fn created_at(&self) -> SystemTime {
        self.created_at
    }

    /// Replace the evidence supporting this binding.
    ///
    /// Assurance is re-evaluated when evidence changes; it is never a one-time
    /// stamp (`ARCHITECTURE.md` §1).
    #[must_use]
    pub fn with_evidence(mut self, evidence: Evidence) -> Self {
        self.evidence = evidence;
        self
    }

    /// Whether control may be driven through this binding.
    ///
    /// This is the only place the question is answered. There is deliberately
    /// no `Run::control_eligibility`: a Run's `session_id` is a structural
    /// reference to its current association, and trusting it would be the
    /// forbidden shape `if run.session_id exists { control_allowed = true }`
    /// (ADR 0002, Q8).
    #[must_use]
    pub fn control_eligibility(&self) -> ControlEligibility {
        if self.assurance().permits_control() {
            ControlEligibility::Eligible
        } else {
            ControlEligibility::AssuranceTooWeak
        }
    }

    /// Whether this binding is a runtime binding that may drive control — the
    /// one a Session may hold at most one of at a time.
    #[must_use]
    pub fn is_control_capable_runtime_binding(&self) -> bool {
        self.kind() == BindingKind::Runtime
            && self.control_eligibility() == ControlEligibility::Eligible
    }
}

/// Whether control may be driven through a binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlEligibility {
    Eligible,
    /// The association is not sure enough to act on. Heuristic evidence never
    /// enables control, however plainly the runtime itself is visible.
    AssuranceTooWeak,
}

impl fmt::Display for BindingKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::ProviderSession => "provider-session",
            Self::Runtime => "runtime",
            Self::Terminal => "terminal",
            Self::History => "history",
        };
        f.write_str(text)
    }
}

#[cfg(test)]
#[path = "binding_tests.rs"]
mod tests;
