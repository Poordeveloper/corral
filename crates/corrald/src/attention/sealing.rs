//! Which provider facts are version-sealed, and the claim a sealed fact makes.
//!
//! Sealing is exact: a claim is sealed for a measured version or an explicitly
//! approved range, never for "the same major.minor still parses"
//! (grill Q13). The table is empty until the acceptance reconciliation seals
//! the matrix's rows, so every hook fact is observed and none asserts a main
//! state — visible, diagnostic, Limited awareness — which is exactly what an
//! unsealed version is entitled to (ADR 0015 D3).

use corral_core::{Assurance, Channel, Claim, Sealing, SemanticState};

use crate::provider::{AgentFactKind, KnownProvider};

/// The claim a provider fact makes, when it makes one.
///
/// A session start and a session end are not turn-state claims; the three
/// turn facts are. `version` is the provider version bound to the runtime the
/// fact came from — `None` when it could not be established, which seals
/// nothing (grill Q12).
#[must_use]
pub fn hook_fact_claim(
    provider: KnownProvider,
    fact: AgentFactKind,
    version: Option<&str>,
    association: Assurance,
    channel: Channel,
) -> Option<Claim> {
    let asserts = match fact {
        AgentFactKind::TurnStarted => SemanticState::Working,
        AgentFactKind::TurnEnded => SemanticState::Ready,
        AgentFactKind::AwaitingInput => SemanticState::NeedsYou,
        AgentFactKind::SessionStarted | AgentFactKind::SessionEnded => return None,
    };
    Some(Claim {
        source: corral_core::EvidenceSource::ProviderHook,
        association,
        channel,
        sealing: sealed(provider, fact, version),
        asserts,
    })
}

/// The sealed rows. None yet: the matrix measured Claude Code 2.1.258 and
/// Codex 0.152.0 and the reconciliation that seals them has not been ruled.
fn sealed(_provider: KnownProvider, _fact: AgentFactKind, version: Option<&str>) -> Sealing {
    match version {
        Some(_) | None => Sealing::Unsealed,
    }
}
