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
    known: HashMap<CorralSessionId, SystemTime>,
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
        self.rows.retain(|(held, _), _| *held != provider);
        self.rows.extend(fresh);
        for (session, last_active) in resolved {
            self.known.insert(session, last_active);
        }
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

    /// When the store last saw a Session Corral holds act, if it has.
    #[must_use]
    pub fn last_active(&self, session: CorralSessionId) -> Option<SystemTime> {
        self.known.get(&session).copied()
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
