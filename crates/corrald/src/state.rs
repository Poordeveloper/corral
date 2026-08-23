use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use corral_state::{FatalState, Refusal, StateError, Store};

/// What the registry said when asked whether it can still vouch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vouched {
    Yes,
    /// Held by another writer for longer than the store waits. Nothing is
    /// wrong with it; the same request may be sent again.
    NotNow,
}

/// The daemon's one handle on durable state.
///
/// One `Store` behind one lock: the registry is the account's shared truth,
/// and two handles to the same file would be two writers to one log.
///
/// The store is synchronous and can wait on another process holding the
/// database, and `corrald` runs one runtime thread — so every call goes to the
/// blocking pool. On the reactor thread a contended registry would stall every
/// other connection, the idle watchdog, and the signal handler along with it.
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
    pub async fn vouch(self: &Arc<Self>) -> Result<Vouched, StateError> {
        match self.off_the_reactor(Store::vouch).await {
            Ok(()) => Ok(Vouched::Yes),
            Err(StateError::Refused(Refusal::Busy { .. })) => Ok(Vouched::NotNow),
            // Contention is the only refusal this call can produce. Any other
            // means this build's understanding of the store is wrong, which is
            // not a state to answer normally from — and it keeps the caller
            // from inheriting a catch-all that would tell a client to retry a
            // request that can never succeed.
            Err(other) => Err(other),
        }
    }

    /// Whether the registry has concluded it can no longer vouch for durable
    /// truth.
    ///
    /// Read from the store itself rather than from anything a connection task
    /// recorded: the conclusion has to survive the task that reached it being
    /// dropped mid-shutdown, or a daemon that stopped over an untrusted store
    /// could still exit as though nothing happened.
    pub fn stopped_vouching(&self) -> bool {
        self.lock().stopped_vouching()
    }

    /// Run one store call on the blocking pool.
    async fn off_the_reactor<T: Send + 'static>(
        self: &Arc<Self>,
        work: impl FnOnce(&mut Store) -> Result<T, StateError> + Send + 'static,
    ) -> Result<T, StateError> {
        let state = Arc::clone(self);
        match tokio::task::spawn_blocking(move || work(&mut state.lock())).await {
            Ok(outcome) => outcome,
            // The call did not complete, so nothing can be said about the
            // store — which is the same position as a store that cannot vouch.
            Err(source) => Err(StateError::Fatal(FatalState::Storage {
                detail: format!("a registry call did not complete: {source}"),
            })),
        }
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
