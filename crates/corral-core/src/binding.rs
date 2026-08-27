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

    /// The key of the managed-runtime binding Corral owns for one Session
    /// (ADR 0008 D1).
    ///
    /// Minted rather than derived: a runtime Corral launched has no external
    /// system to have named it, and the two identities that would otherwise be
    /// reached for are both forbidden — a pid is never identity, and a
    /// `RunId` names one concrete occurrence rather than the binding that
    /// outlives it (D2).
    ///
    /// One per Session, reused by a resume or a replacement Run. Minting a
    /// second is what the store refuses.
    #[must_use]
    pub fn mint_managed_runtime(node: NodeId) -> Self {
        Self::new(
            node,
            BindingKind::Runtime,
            ProviderId::corral(),
            ExternalId::mint(),
        )
    }
}

/// How a binding sits wrongly in the reserved `corral` provider namespace
/// (ADR 0008 D3).
///
/// Only the two ways of being wrong, because "respected" is the absence of one
/// of these: a type that could also say nothing was wrong would make a refusal
/// representable that has nothing to refuse.
///
/// The rule is directional rather than a blanket refusal of the string,
/// because the Corral-owned runtime binding is precisely the thing that needs
/// it. Stating it as a ban would have been simpler and wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReservedNamespaceMisuse {
    /// A `CorralCreated` runtime binding that does not carry the reserved id.
    /// Without it, what the binding's identity means rests on convention, and
    /// the first provider phase is where conventions go.
    ManagedRuntimeWithoutIt,
    /// Anything else carrying it. Provider-derived identity never occupies the
    /// namespace whose whole meaning is "Corral minted this name".
    ClaimedByAnotherIdentity,
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
    identity_status: IdentityStatus,
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
            // A binding is created by evidence Corral accepted, so it starts
            // out as the identity Corral stands behind. Contest is a later
            // fact about it and never a shape it can be born in
            // (ADR 0004 D8).
            identity_status: IdentityStatus::Confirmed,
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

    /// Whether Corral still stands behind the external identity this edge
    /// names.
    #[must_use]
    pub fn identity_status(&self) -> IdentityStatus {
        self.identity_status
    }

    /// The same edge, with its identity claim contested.
    ///
    /// Separate from evidence because a contest is not weaker evidence about
    /// the same claim: it is positive evidence that two incompatible identity
    /// claims were observed, and an assurance downgrade would misdescribe it
    /// (ADR 0004 D8). Monotonic — nothing here returns a contested edge to
    /// `Confirmed`, and clearing one needs a correction mechanism no accepted
    /// decision yet describes.
    #[must_use]
    pub fn contested(mut self) -> Self {
        self.identity_status = IdentityStatus::Contested;
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

    /// Whether this binding may authorize continuing the provider's own
    /// session — the authority `session.resume` derives from an identity
    /// claim, and nothing else.
    ///
    /// Deliberately not folded into `control_eligibility`. That answer is
    /// generic binding-control eligibility, and an `IdentityContested` arm
    /// there would invite `!= Eligible → disable everything` — disabling Open
    /// and attach, which ride the Deterministic runtime binding and are
    /// untouched by a contest (ADR 0004 D8; founder emphasis, R2 Q2).
    ///
    /// The runtime preconditions a resume also has — no live Run, and a
    /// previous Run whose exit is established — are not facts a binding holds,
    /// so they are answered where they are known and never here.
    #[must_use]
    pub fn native_resume_eligibility(&self) -> NativeResumeEligibility {
        match (self.identity_status, self.assurance().permits_control()) {
            // Checked before assurance because a contested binding keeps the
            // assurance it earned: Attested-and-contested is not Heuristic,
            // and reporting it as too weak would name the wrong repair.
            (IdentityStatus::Contested, _) => NativeResumeEligibility::IdentityContested,
            (IdentityStatus::Confirmed, false) => NativeResumeEligibility::AssuranceTooWeak,
            (IdentityStatus::Confirmed, true) => NativeResumeEligibility::Eligible,
        }
    }

    /// Whether this binding is a runtime binding that may drive control — the
    /// one a Session may hold at most one of at a time.
    #[must_use]
    pub fn is_control_capable_runtime_binding(&self) -> bool {
        self.kind() == BindingKind::Runtime
            && self.control_eligibility() == ControlEligibility::Eligible
    }

    /// Whether this binding respects the reserved `corral` provider namespace.
    ///
    /// Asked of provenance and kind rather than of assurance: what the
    /// namespace records is who minted the identity, which is settled when the
    /// edge is created and never re-evaluated (ADR 0008 D3).
    pub fn reserved_namespace(&self) -> Result<(), ReservedNamespaceMisuse> {
        let managed_runtime =
            self.kind() == BindingKind::Runtime && self.provenance == Provenance::CorralCreated;
        match (
            managed_runtime,
            self.key.provider().is_reserved_for_corral(),
        ) {
            (true, true) | (false, false) => Ok(()),
            (true, false) => Err(ReservedNamespaceMisuse::ManagedRuntimeWithoutIt),
            (false, true) => Err(ReservedNamespaceMisuse::ClaimedByAnotherIdentity),
        }
    }
}

/// Whether control may be driven through a binding.
///
/// Generic binding-control eligibility, and it stays generic: the operations
/// that depend on a *provider identity claim* ask
/// `Binding::native_resume_eligibility` instead (ADR 0004 D8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlEligibility {
    Eligible,
    /// The association is not sure enough to act on. Heuristic evidence never
    /// enables control, however plainly the runtime itself is visible.
    AssuranceTooWeak,
}

/// Whether Corral still stands behind the external identity a binding names.
///
/// Orthogonal to assurance, and that orthogonality is the point: an Attested
/// binding whose identity is contested is still Attested, because what became
/// unsafe is the claim about *which* external identity this is, not how the
/// association was learned (ADR 0004 D8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IdentityStatus {
    Confirmed,
    /// Contradictory provider-identity evidence has been observed. Monotonic
    /// in this phase: no accepted fact returns a binding to `Confirmed`.
    Contested,
}

/// Whether the provider's own session may be continued through this binding.
///
/// Operation-specific by construction. It answers one question — may a
/// provider external id be placed into a native resume — and a consumer that
/// wanted a different question answered has to ask a different one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeResumeEligibility {
    Eligible,
    /// The association is not sure enough to continue the provider's session
    /// under it.
    AssuranceTooWeak,
    /// Two incompatible identity claims have been observed, so Corral does not
    /// know which provider session this names. No external id reaches a resume
    /// argv from here.
    IdentityContested,
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
