//! The history rows a daemon shows: sessions a provider's store holds that
//! resolve to no Session Corral has — live state, minted a stable id per
//! identity for the row, durable only at continuation (ADR 0016 D2).

use std::collections::HashMap;
use std::time::SystemTime;

use corral_core::{CorralSessionId, ExternalId};

use super::HistoryEntry;
use crate::provider::KnownProvider;

/// Whether a pass's answers were still current when it published them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Published {
    Installed,
    /// An identity was claimed while this pass was resolving, so its answers
    /// are dropped rather than installed. The next pass reads the store as it
    /// now stands.
    Stale,
}

/// One unresolved entry as the list shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryRow {
    /// The identity this row is listed under. Minted here and stable across
    /// passes; never a Session in the registry until continuation.
    pub session: CorralSessionId,
    pub entry: HistoryEntry,
}

/// Every provider's rows, and the recency the store recorded for Sessions
/// Corral already holds.
#[derive(Debug, Default)]
pub struct HistoryRows {
    rows: HashMap<(KnownProvider, ExternalId), HistoryRow>,
    /// The entries that resolved to a Session Corral already holds, under
    /// that Session's own id.
    ///
    /// Listed like any other row when nothing live is showing that Session:
    /// a managed session that exited yesterday and its history file are one
    /// row (ADR 0016 D2), and a daemon restart forgets every runtime, so
    /// without these a session would vanish from the list by having been
    /// continued once. Part of the provider's snapshot, not a fact of its
    /// own: a file that is deleted or ages out of the window stops saying
    /// anything, and a map only ever added to could never say so.
    known: HashMap<(KnownProvider, ExternalId), HistoryRow>,
    /// Bumped whenever an identity stops being one a history row may stand
    /// for. A pass resolves each entry against the registry one at a time and
    /// publishes the lot at the end; a continuation that lands in between has
    /// already given that identity a Session, and the pass's answer for it is
    /// from before that. Without this, republishing the stale answer mints a
    /// second id for a provider session that now has one (ADR 0016 D2).
    generation: u64,
}

impl HistoryRows {
    /// What the rows were resolved against, to be handed back to `replace`.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Replace one provider's rows with a fresh pass, keeping the ids of the
    /// identities that were already listed.
    ///
    /// Refused when an identity was claimed while the pass was resolving:
    /// those answers are from before the claim, and the next pass reads a
    /// store that already holds it.
    pub fn replace(
        &mut self,
        provider: KnownProvider,
        unresolved: Vec<HistoryEntry>,
        resolved: Vec<(CorralSessionId, HistoryEntry)>,
        resolved_at: u64,
    ) -> Published {
        if resolved_at != self.generation {
            return Published::Stale;
        }
        let mut fresh = HashMap::new();
        for entry in unresolved {
            let key = (provider, entry.external_id.clone());
            let session = self
                .rows
                .get(&key)
                .map_or_else(CorralSessionId::mint, |row| row.session);
            fresh.insert(key, HistoryRow { session, entry });
        }
        self.retract(provider);
        self.rows.extend(fresh);
        self.known
            .extend(resolved.into_iter().map(|(session, entry)| {
                (
                    (provider, entry.external_id.clone()),
                    HistoryRow { session, entry },
                )
            }));
        Published::Installed
    }

    /// Drop everything one provider's store was the evidence for.
    ///
    /// A pass that cannot read a store under a layout the matrix sealed at the
    /// version installed *now* is not a pass that says nothing; it says the
    /// evidence these rows stood on is gone (ADR 0016 D1). Keeping them would
    /// let a row learned under a sealed version stay listable — and
    /// continuable — after an in-place upgrade to an unmeasured one.
    pub fn retract(&mut self, provider: KnownProvider) {
        self.rows.retain(|(held, _), _| *held != provider);
        self.known.retain(|(held, _), _| *held != provider);
    }

    /// Drop one row, because the identity it stood for is a Session now.
    /// The next pass would resolve it anyway; forgetting it here keeps the
    /// list from showing the row and its own Session at once.
    pub fn forget(&mut self, provider: KnownProvider, external_id: &ExternalId) {
        self.rows.remove(&(provider, external_id.clone()));
        // Any resolution taken before this moment answered for an identity
        // nothing had claimed, and one now does.
        self.generation = self.generation.wrapping_add(1);
    }

    /// Every row the stores hold, newest first — the identities Corral has
    /// no Session for, and the Sessions it does whose evidence is the store.
    ///
    /// The caller drops the ones a live tier is already showing: this answers
    /// what the stores hold, not what is missing from the list above it.
    #[must_use]
    pub fn rows(&self) -> Vec<HistoryRow> {
        let mut rows: Vec<HistoryRow> = self
            .rows
            .values()
            .chain(self.known.values())
            .cloned()
            .collect();
        rows.sort_by(|a, b| {
            b.entry
                .last_active
                .cmp(&a.entry.last_active)
                .then_with(|| a.session.to_string().cmp(&b.session.to_string()))
        });
        rows
    }

    /// When a store last saw a Session Corral holds act, if one has.
    ///
    /// The newest across providers rather than one provider's answer: a
    /// Session can hold a binding in more than one store, and the question is
    /// when it last acted, not where.
    #[must_use]
    pub fn last_active(&self, session: CorralSessionId) -> Option<SystemTime> {
        self.known
            .values()
            .filter(|row| row.session == session)
            .map(|row| row.entry.last_active)
            .max()
    }

    /// The row listed under this id, if it is an identity with no Session.
    ///
    /// Only the unresolved ones: a row that resolved already has a Session,
    /// and a continuation of it is that Session's, decided from what the
    /// registry holds rather than from a store observation.
    #[must_use]
    pub fn row(&self, session: CorralSessionId) -> Option<&HistoryRow> {
        self.rows.values().find(|row| row.session == session)
    }
}

#[cfg(test)]
#[path = "rows_tests.rs"]
mod tests;
