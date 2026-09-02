//! Every Session's evidence and derived state, in one place the daemon owns.
//!
//! The engine's pure function needs a ledger of claims and a clock; the
//! tracker needs to remember the last state. This holds both for every
//! Session the daemon knows, assigns the observation sequence "newest" is
//! measured by (grill Q3), and turns a tick into the list of changes the
//! journal records.

use std::collections::HashMap;
use std::time::SystemTime;

use corral_core::{
    AttentionItemId, AttentionReason, AttentionState, Claim, CorralSessionId, EvidenceSource,
    MainState, SemanticState,
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
    /// The claim the new state rests on, when a claim decided it.
    pub decided_by: Option<Claim>,
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
    pub fn observe(&mut self, session: CorralSessionId, claim: Claim, observed_at: SystemTime) {
        self.ordinal += 1;
        let observed = Observed {
            claim,
            observed_at,
            ordinal: self.ordinal,
        };
        let tracked = self
            .sessions
            .entry(session)
            .or_insert_with(|| Tracked::new(observed_at));
        tracked.claims.retain(|held| {
            (held.claim.source, held.claim.asserts) != (claim.source, claim.asserts)
        });
        tracked.claims.push(observed);
    }

    /// Re-derive every Session at `now` and report what changed.
    pub fn tick(
        &mut self,
        now: SystemTime,
        execution: impl Fn(CorralSessionId) -> ExecutionState,
    ) -> Vec<Change> {
        let mut changes = Vec::new();
        for (session, tracked) in &mut self.sessions {
            let derived = derive(execution(*session), &tracked.claims, &self.horizons, now);
            let from = tracked.attention.state().main();
            let transition = tracked.attention.apply(derived, now);
            if transition == Transition::Unchanged {
                continue;
            }
            let decided_by = SemanticState::try_from(derived.main)
                .ok()
                .and_then(|state| {
                    tracked
                        .claims
                        .iter()
                        .filter(|held| held.claim.asserts == state)
                        .max_by_key(|held| held.ordinal)
                        .map(|held| held.claim)
                });
            changes.push(Change {
                session: *session,
                from,
                to: derived.main,
                transition,
                decided_by,
                at: now,
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

    /// The last activity claim's instant, so the screen thread's publication
    /// is turned into a claim once per new byte and not once per tick.
    #[must_use]
    pub fn last_activity(&self, session: CorralSessionId) -> Option<SystemTime> {
        self.sessions.get(&session).and_then(|tracked| {
            tracked
                .claims
                .iter()
                .filter(|held| held.claim.source == EvidenceSource::PtyActivity)
                .map(|held| held.observed_at)
                .max()
        })
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
