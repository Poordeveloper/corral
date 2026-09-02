use crate::assurance::Assurance;
use crate::attention_state::SemanticState;
use crate::evidence::EvidenceSource;

/// One semantic claim as a source makes it, before the engine weighs it.
///
/// Entitlement has two axes (ADR 0015 D3): whether the evidence is about this
/// Session at all — `association`, the binding's assurance — and whether this
/// source may say this — its class, the channel it observed, and whether the
/// matrix sealed the interpretation for the producing version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Claim {
    pub source: EvidenceSource,
    pub association: Assurance,
    pub channel: Channel,
    pub sealing: Sealing,
    pub asserts: SemanticState,
}

/// Where the observation was made.
///
/// Screen and activity evidence exist only on a PTY Corral owns; an external
/// runtime has no stream Corral reads, and nothing stands in for it
/// (ADR 0015 D5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Channel {
    CorralOwnedPty,
    ExternalRuntime,
}

/// Whether the matrix sealed this interpretation for the version that
/// produced the evidence (grill Q13, Q14).
///
/// An unsealed rule or event loads, counts, and asserts nothing user-visible;
/// it is not a weaker claim, it is no claim.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sealing {
    Sealed,
    Unsealed,
}

/// Whether a claim may reach a main state, and if not, which axis refused it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Entitlement {
    Entitled,
    /// The binding is Heuristic: secondary metadata only, never a main state,
    /// never an item (AGENTS.md §Core model).
    AssociationTooWeak,
    /// This class of evidence may not assert this state, whatever it saw.
    SourceMayNotAssert,
    /// The interpretation is not sealed for the producing version.
    Unsealed,
    /// The source needs a PTY Corral owns, and this runtime is not one.
    NotCorralOwned,
}

impl Claim {
    /// ADR 0015 D3's table, applied.
    ///
    /// Association is judged first: a claim nobody is entitled to make about
    /// this Session is refused before its source is consulted, so a Heuristic
    /// row never learns what it could have said.
    #[must_use]
    pub fn entitlement(&self) -> Entitlement {
        if !self.association.permits_control() {
            return Entitlement::AssociationTooWeak;
        }
        match self.source {
            EvidenceSource::PtyActivity => match (self.channel, self.asserts) {
                (Channel::ExternalRuntime, _) => Entitlement::NotCorralOwned,
                (Channel::CorralOwnedPty, SemanticState::Working) => Entitlement::Entitled,
                (Channel::CorralOwnedPty, SemanticState::NeedsYou | SemanticState::Ready) => {
                    Entitlement::SourceMayNotAssert
                }
            },
            EvidenceSource::ScreenDetection => match (self.channel, self.asserts, self.sealing) {
                (Channel::ExternalRuntime, _, _) => Entitlement::NotCorralOwned,
                // Screen Working stays diagnostic in this phase: activity and
                // hooks carry Working, and "looks busy" has the blurriest
                // edge (grill Q14).
                (_, SemanticState::Working, _) => Entitlement::SourceMayNotAssert,
                (_, SemanticState::NeedsYou | SemanticState::Ready, Sealing::Unsealed) => {
                    Entitlement::Unsealed
                }
                (_, SemanticState::NeedsYou | SemanticState::Ready, Sealing::Sealed) => {
                    Entitlement::Entitled
                }
            },
            EvidenceSource::InBandSignal => match (self.channel, self.sealing) {
                (Channel::ExternalRuntime, _) => Entitlement::NotCorralOwned,
                (Channel::CorralOwnedPty, Sealing::Unsealed) => Entitlement::Unsealed,
                (Channel::CorralOwnedPty, Sealing::Sealed) => Entitlement::Entitled,
            },
            // A received event is sufficient for exactly the claim it denotes,
            // on either channel: delivery is unreliable, its semantics are not
            // (grill Q2).
            EvidenceSource::ProviderHook => match self.sealing {
                Sealing::Sealed => Entitlement::Entitled,
                Sealing::Unsealed => Entitlement::Unsealed,
            },
            EvidenceSource::CorralConstructed
            | EvidenceSource::NodeRuntimeObservation
            | EvidenceSource::HistoryRecord
            | EvidenceSource::Correlation
            | EvidenceSource::UserAssertion => Entitlement::SourceMayNotAssert,
        }
    }
}

#[cfg(test)]
#[path = "entitlement_tests.rs"]
mod tests;
