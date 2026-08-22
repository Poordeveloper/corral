use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use corral_state::{StateError, Store};

/// The daemon's one handle on durable state.
///
/// One `Store` behind one lock: the registry is the account's shared truth,
/// and two handles to the same file would be two writers to one log. Nothing
/// here is held across an await, so a slow client cannot block the store.
pub struct DaemonState {
    store: Mutex<Store>,
}

impl DaemonState {
    /// Open and validate the registry.
    ///
    /// Called before the daemon binds its endpoint, so a store that cannot be
    /// used is a startup failure rather than something discovered a
    /// millisecond after a client's hello succeeded (ADR 0002, Q14).
    pub fn open(registry: &Path) -> Result<Self, StateError> {
        Ok(Self {
            store: Mutex::new(Store::open(registry)?),
        })
    }

    /// Confirm the registry can still vouch for durable truth.
    ///
    /// What an answer derived from the registry needs before it may be given.
    /// Protocol 1 assigns no session encoding, so nothing this build serves
    /// carries a fact out of the store — but an empty list is still a claim
    /// about it, and this is the question behind that claim.
    pub fn vouch(&self) -> Result<(), StateError> {
        self.lock().vouch()
    }

    /// A poisoned lock means another task panicked while holding it. The store
    /// itself decides whether it can still vouch for durable truth, and it
    /// answers every caller the same way once it cannot — so refusing to look
    /// would replace that answer with a stuck daemon.
    fn lock(&self) -> MutexGuard<'_, Store> {
        self.store
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}
