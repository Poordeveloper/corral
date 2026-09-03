//! The history rows a daemon shows: sessions a provider's store holds that
//! resolve to no Session Corral has — live state, minted a stable id per
//! identity for the row, durable only at continuation (ADR 0016 D2).

use std::collections::HashMap;
use std::time::SystemTime;

use corral_core::{CorralSessionId, ExternalId};

use super::HistoryEntry;
use crate::provider::KnownProvider;

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
    /// Keyed by the provider that observed it, because it is part of that
    /// provider's snapshot and not a fact of its own: a session whose file is
    /// deleted or ages out of the window has no recency any more, and a map
    /// only ever added to could never say so.
    known: HashMap<(KnownProvider, CorralSessionId), SystemTime>,
}

impl HistoryRows {
    /// Replace one provider's rows with a fresh pass, keeping the ids of the
    /// identities that were already listed.
    pub fn replace(
        &mut self,
        provider: KnownProvider,
        unresolved: Vec<HistoryEntry>,
        resolved: Vec<(CorralSessionId, SystemTime)>,
    ) {
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
        self.known.extend(
            resolved
                .into_iter()
                .map(|(session, last_active)| ((provider, session), last_active)),
        );
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
    }

    /// The rows, newest first.
    #[must_use]
    pub fn rows(&self) -> Vec<HistoryRow> {
        let mut rows: Vec<HistoryRow> = self.rows.values().cloned().collect();
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
            .iter()
            .filter(|((_, held), _)| *held == session)
            .map(|(_, last_active)| *last_active)
            .max()
    }

    /// The row listed under this id, if it is one of ours.
    #[must_use]
    pub fn row(&self, session: CorralSessionId) -> Option<&HistoryRow> {
        self.rows.values().find(|row| row.session == session)
    }
}

#[cfg(test)]
#[path = "rows_tests.rs"]
mod tests;
