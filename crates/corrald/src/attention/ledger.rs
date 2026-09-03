//! Every Session's evidence and derived state, in one place the daemon owns.
//!
//! The engine's pure function needs a ledger of claims and a clock; the
//! tracker needs to remember the last state. This holds both for every
//! Session the daemon knows, assigns the observation sequence "newest" is
//! measured by (grill Q3), and turns a tick into the list of changes the
//! journal records.

use std::collections::HashMap;
use std::time::{Duration, SystemTime};

use crate::clock::Reading;

use corral_core::{
    AttentionItemId, AttentionReason, AttentionState, Claim, CorralSessionId, MainState,
};
use corral_protocol::method::{AttentionCount, AttentionSummaryResult};

use super::{Acknowledgement, Horizons, Item, Observed, SessionAttention, Transition, derive};
use crate::runtime::ExecutionState;

/// One derivation's outcome, as the journal and the notifier read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Change {
    pub session: CorralSessionId,
    pub from: MainState,
    pub to: MainState,
    pub transition: Transition,
    /// The claim the new state rests on, when a claim decided it — or the
    /// one that rotted, when a horizon did.
    pub decided_by: Option<Claim>,
    /// The horizon that claim lives under (grill Q15).
    pub horizon: Option<Duration>,
    /// How far past that horizon the claim had run when the clock noticed.
    /// Only a rot has one: it is the difference between the horizon a claim
    /// was configured for and the moment its expiry was acted on.
    pub expired_after: Option<Duration>,
    pub at: SystemTime,
}

struct Tracked {
    attention: SessionAttention,
    /// The newest observation per (source, asserted state). A source
    /// re-asserting the same state replaces its earlier claim — the screen
    /// still showing the blocker is one claim, observed again — and never
    /// accumulates.
    claims: Vec<Observed>,
}

/// The daemon's attention ledger.
pub struct Ledger {
    horizons: Horizons,
    ordinal: u64,
    sessions: HashMap<CorralSessionId, Tracked>,
}

impl Default for Ledger {
    fn default() -> Self {
        Self::new(Horizons::default())
    }
}

impl Ledger {
    #[must_use]
    pub fn new(horizons: Horizons) -> Self {
        Self {
            horizons,
            ordinal: 0,
            sessions: HashMap::new(),
        }
    }

    /// Record one claim about a Session, as the next observation in the
    /// daemon's sequence.
    pub fn observe(&mut self, session: CorralSessionId, claim: Claim, at: Reading) {
        let observed_at = at.mono;
        let ordinal = self.ordinal + 1;
        let tracked = self
            .sessions
            .entry(session)
            .or_insert_with(|| Tracked::new(at.wall));
        // A claim presented again without having been established again is the
        // same fact, not a later one. The daemon polls its sources on a clock,
        // and an unchanged reading taking a newer place in the sequence would
        // let polling decide which claim contradicted which — the one thing
        // the sequence exists to answer (grill Q3).
        if tracked
            .claims
            .iter()
            .any(|held| held.claim == claim && observed_at <= held.observed_at)
        {
            return;
        }
        tracked.claims.retain(|held| {
            (held.claim.source, held.claim.asserts) != (claim.source, claim.asserts)
        });
        tracked.claims.push(Observed {
            claim,
            observed_at,
            ordinal,
        });
        self.ordinal = ordinal;
    }

    /// Re-derive every Session at `now` and report what changed.
    pub fn tick(
        &mut self,
        now: Reading,
        execution: impl Fn(CorralSessionId) -> ExecutionState,
    ) -> Vec<Change> {
        let mut changes = Vec::new();
        for (session, tracked) in &mut self.sessions {
            let derived = derive(execution(*session), &tracked.claims, &self.horizons, now);
            let from = tracked.attention.state().main();
            let transition = tracked.attention.apply(derived, now.wall);
            if transition == Transition::Unchanged {
                continue;
            }
            let horizon = derived.rests_on.map(|observed| {
                self.horizons
                    .of(observed.claim.source, observed.claim.asserts)
            });
            // A rot is the one transition whose cause is the passage of time,
            // so it is the one that has a distance past the horizon to report.
            let expired_after = match (derived.main, derived.rests_on, horizon) {
                (MainState::Unknown, Some(observed), Some(horizon)) => now
                    .mono
                    .since(observed.observed_at)
                    .and_then(|age| age.checked_sub(horizon)),
                _ => None,
            };
            changes.push(Change {
                session: *session,
                from,
                to: derived.main,
                transition,
                decided_by: derived.rests_on.map(|observed| observed.claim),
                horizon,
                expired_after,
                at: now.wall,
            });
        }
        changes
    }

    #[must_use]
    pub fn state(&self, session: CorralSessionId) -> Option<(AttentionState, Option<Item>)> {
        self.sessions
            .get(&session)
            .map(|tracked| (tracked.attention.state(), tracked.attention.item()))
    }

    /// The daemon's projection of current items: totals and unacknowledged
    /// per class, never a state of its own (grill Q23).
    #[must_use]
    pub fn summary(&self) -> AttentionSummaryResult {
        let mut needs_you = AttentionCount {
            total: 0,
            unacknowledged: 0,
        };
        let mut ready = AttentionCount {
            total: 0,
            unacknowledged: 0,
        };
        for tracked in self.sessions.values() {
            let Some(item) = tracked.attention.item() else {
                continue;
            };
            let count = match item.reason() {
                AttentionReason::NeedsInput => &mut needs_you,
                AttentionReason::TurnComplete => &mut ready,
                AttentionReason::RuntimeEnded => continue,
            };
            count.total += 1;
            if !item.acknowledged() {
                count.unacknowledged += 1;
            }
        }
        AttentionSummaryResult { needs_you, ready }
    }

    pub fn acknowledge(
        &mut self,
        session: CorralSessionId,
        item: AttentionItemId,
    ) -> Acknowledgement {
        match self.sessions.get_mut(&session) {
            Some(tracked) => tracked.attention.acknowledge(item),
            None => Acknowledgement::NoCurrentItem,
        }
    }

    /// Open succeeded for this Session (grill Q18).
    pub fn opened(&mut self, session: CorralSessionId) {
        if let Some(tracked) = self.sessions.get_mut(&session) {
            tracked.attention.opened();
        }
    }

    /// The claims held for a Session, newest last. Diagnostics and tests;
    /// derivation reads the ledger through `tick`.
    #[must_use]
    pub fn claims(&self, session: CorralSessionId) -> Vec<Claim> {
        self.sessions
            .get(&session)
            .map(|tracked| {
                let mut held = tracked.claims.clone();
                held.sort_by_key(|observed| observed.ordinal);
                held.into_iter().map(|observed| observed.claim).collect()
            })
            .unwrap_or_default()
    }
}

impl Tracked {
    fn new(created_at: SystemTime) -> Self {
        Self {
            attention: SessionAttention::new(created_at),
            claims: Vec::new(),
        }
    }
}

#[cfg(test)]
#[path = "ledger_tests.rs"]
mod tests;
