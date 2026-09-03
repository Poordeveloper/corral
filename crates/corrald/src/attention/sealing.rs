//! Which provider facts are version-sealed, and the claim a sealed fact makes.
//!
//! Sealing is exact: a claim is sealed for a measured version or an explicitly
//! approved range, never for "the same major.minor still parses"
//! (grill Q13). A fact this table does not seal is still observed, still
//! journaled, and asserts no main state — Limited awareness, which is exactly
//! what an unmeasured version is entitled to (ADR 0015 D3). Sealing is a human
//! act on evidence: this table lands by human merge under
//! `HUMAN_REVIEW_REQUIRED`, and its evidence is
//! `docs/evidence/pr8-attention-semantics-2026-09-03.md` (ADR 0015 D9).

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

/// The sealed rows.
///
/// One row per (provider, version, fact) that a capture actually establishes.
/// Not a range: Claude Code 2.1.258 and 2.1.259 both exist here for the store
/// layout, but only 2.1.258 carries the hook semantics, because the second run
/// measured compaction and failure rather than the turn events — and 2.1.258's
/// binary is gone, so nothing can be re-measured on it (matrix, second run).
///
/// `version` is the version bound to the runtime the fact came from. `None`
/// means the binding could not be established, which seals nothing: not
/// knowing which build produced a fact is not a licence to read it as a
/// measured one (grill Q12).
fn sealed(provider: KnownProvider, fact: AgentFactKind, version: Option<&str>) -> Sealing {
    let Some(version) = version else {
        return Sealing::Unsealed;
    };
    let measured = match (provider, version) {
        // Claude Code 2.1.258: `UserPromptSubmit` opens a turn, `Stop` ends
        // one, and `Notification(permission_prompt)` accompanies a pending
        // `PermissionRequest` — matrix C1–C3, C5–C7, C9, C10.
        (KnownProvider::Claude, "2.1.258") => matches!(
            fact,
            AgentFactKind::TurnStarted | AgentFactKind::TurnEnded | AgentFactKind::AwaitingInput
        ),
        // Codex 0.152.0 notifies `agent-turn-complete` and nothing else: it
        // has no turn-start event, and its approval request is announced on
        // the screen and in the OSC title rather than out of band — matrix
        // X1–X5, X7.
        (KnownProvider::Codex, "0.152.0") => fact == AgentFactKind::TurnEnded,
        _ => false,
    };
    if measured {
        Sealing::Sealed
    } else {
        Sealing::Unsealed
    }
}

#[cfg(test)]
#[path = "sealing_tests.rs"]
mod tests;
