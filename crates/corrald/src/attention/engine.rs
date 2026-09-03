//! The pure part of the engine: a ledger of observed claims and a clock in,
//! a main state out (ADR 0015 D2–D4).
//!
//! Nothing here remembers anything. The per-Session tracker feeds it the
//! claims it holds and applies the result; this function only says what those
//! claims, taken together at this instant, entitle Corral to assert.

use std::time::{Duration, SystemTime};

use corral_core::{
    AttentionState, Claim, Entitlement, EvidenceSource, LastKnown, MainState, SemanticState,
};

use crate::runtime::ExecutionState;

/// One claim as the engine holds it: what was claimed, when the daemon saw
/// it, and where that sits in the daemon's own observation sequence.
///
/// The ordinal is what "newest" means (grill Q3). Wall clocks are compared
/// only against horizons, never against each other across sources.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Observed {
    pub claim: Claim,
    pub observed_at: SystemTime,
    pub ordinal: u64,
}

/// What derivation concluded, before the tracker decides since when.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Derived {
    pub main: MainState,
    /// The last reliable fact, carried only when `main` is Unknown.
    pub last_known: Option<LastKnown>,
    /// The observation the state rests on: the claim that decided it, or —
    /// beneath an Unknown a horizon caused — the claim that rotted. `None`
    /// where no claim is involved at all: an ended runtime, or one whose
    /// execution Corral cannot place.
    ///
    /// Derivation says which claim carried the state because derivation is
    /// what knows. A later search for "the newest claim asserting this
    /// state" would answer with evidence entitlement refused, which is
    /// exactly the opposite of what happened (ADR 0015 D8).
    pub rests_on: Option<Observed>,
}

impl Derived {
    /// The state a client reads, entered at `since`.
    #[must_use]
    pub fn into_state(self, since: SystemTime) -> AttentionState {
        match self.main {
            MainState::Unknown => AttentionState::unknown(since, self.last_known),
            main => AttentionState::asserted(main, since),
        }
    }
}

/// How long each kind of claim stays a claim.
///
/// Policy defaults, not contract (grill Q15): the contract is that every
/// semantic claim has one and that none is widened to make Unknown rarer.
/// Screen and in-band claims are re-stamped by the screen thread for as long
/// as the screen still supports them, so their horizon only has to outlast
/// that re-observation cadence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Horizons {
    pub activity_quiet: Duration,
    pub screen_reobservation: Duration,
    pub hook_working: Duration,
    pub hook_needs_you: Duration,
    pub hook_ready: Duration,
}

impl Default for Horizons {
    fn default() -> Self {
        Self {
            activity_quiet: Duration::from_secs(3),
            screen_reobservation: Duration::from_secs(5),
            hook_working: Duration::from_secs(15 * 60),
            hook_needs_you: Duration::from_secs(5 * 60),
            hook_ready: Duration::from_secs(2 * 60 * 60),
        }
    }
}

impl Horizons {
    /// The horizon a claim from this source, asserting this state, lives under.
    #[must_use]
    pub fn of(&self, source: EvidenceSource, asserts: SemanticState) -> Duration {
        match source {
            EvidenceSource::PtyActivity => self.activity_quiet,
            EvidenceSource::ScreenDetection | EvidenceSource::InBandSignal => {
                self.screen_reobservation
            }
            EvidenceSource::ProviderHook
            | EvidenceSource::CorralConstructed
            | EvidenceSource::NodeRuntimeObservation
            | EvidenceSource::HistoryRecord
            | EvidenceSource::Correlation
            | EvidenceSource::UserAssertion => match asserts {
                SemanticState::Working => self.hook_working,
                SemanticState::NeedsYou => self.hook_needs_you,
                SemanticState::Ready => self.hook_ready,
            },
        }
    }
}

/// ADR 0015 D2–D4, applied at one instant.
#[must_use]
pub fn derive(
    execution: ExecutionState,
    claims: &[Observed],
    horizons: &Horizons,
    now: SystemTime,
) -> Derived {
    // Execution gates semantics (D2): an ended runtime has nothing left to
    // be Needs You about, and a runtime Corral cannot place is not one it
    // may describe.
    if execution == ExecutionState::Exited {
        return Derived {
            main: MainState::Exited,
            last_known: None,
            rests_on: None,
        };
    }

    let entitled: Vec<&Observed> = claims
        .iter()
        .filter(|observed| observed.claim.entitlement() == Entitlement::Entitled)
        .collect();
    // A claim nobody was entitled to make was never a fact, so it is not
    // even the last known one.
    let last_known = entitled
        .iter()
        .max_by_key(|observed| observed.ordinal)
        .map(|observed| LastKnown::new(observed.claim.asserts.into(), observed.observed_at));

    if execution == ExecutionState::Unknown {
        // Not a rot: the claims may be perfectly fresh. What cannot be
        // placed is the runtime.
        return Derived {
            main: MainState::Unknown,
            last_known,
            rests_on: None,
        };
    }

    // Contradiction is an ordering fact, not a freshness one: once a later
    // entitled claim asserted a different state, the earlier one is over, and
    // the later claim's own rot does not bring it back. Activity is the one
    // source that contradicts nothing — the flow that draws a blocker is the
    // flow that would otherwise read as work — so it neither supersedes nor
    // is superseded here.
    let standing: Vec<&Observed> = entitled
        .iter()
        .copied()
        .filter(|observed| {
            !entitled.iter().any(|other| {
                other.claim.source != EvidenceSource::PtyActivity
                    && other.ordinal > observed.ordinal
                    && other.claim.asserts != observed.claim.asserts
            })
        })
        .collect();

    let fresh: Vec<&Observed> = standing
        .iter()
        .copied()
        .filter(|observed| {
            let horizon = horizons.of(observed.claim.source, observed.claim.asserts);
            now.duration_since(observed.observed_at)
                .is_ok_and(|age| age <= horizon)
        })
        .collect();

    let Some(newest) = fresh.iter().max_by_key(|observed| observed.ordinal) else {
        // Nothing is fresh, so the newest entitled claim is the one whose
        // horizon ran out: the rot the journal records by how far past it ran.
        return Derived {
            main: MainState::Unknown,
            last_known,
            rests_on: entitled
                .iter()
                .max_by_key(|observed| observed.ordinal)
                .map(|observed| **observed),
        };
    };

    // Activity is the default and a blocker the exception (D4): the prompt
    // that blocks the agent is drawn by the same output flow that would
    // otherwise read as work. Only a blocker still standing is one, so
    // activity never revives one a later claim cleared.
    let blocker = fresh
        .iter()
        .filter(|observed| observed.claim.asserts == SemanticState::NeedsYou)
        .max_by_key(|observed| observed.ordinal);
    let rests_on = match blocker {
        Some(blocker) if newest.claim.source == EvidenceSource::PtyActivity => **blocker,
        _ => **newest,
    };
    Derived {
        main: rests_on.claim.asserts.into(),
        last_known: None,
        rests_on: Some(rests_on),
    }
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
