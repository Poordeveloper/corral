use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use corral_state::{FatalState, Refusal, StateError, Store};

use crate::runtime::{AttachTokens, ManagedSessions};

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
    /// Set when a store call could not complete at all. The store latches its
    /// own conclusions, but it never saw this one, and the exit status is read
    /// from here.
    unreachable: AtomicBool,
    /// The sessions this daemon is running, and the tokens it has issued for
    /// their terminals.
    ///
    /// Live runtime state, deliberately beside the store rather than in it:
    /// a running process is runtime-owned truth and is never persisted as
    /// fact (AGENTS.md §Durable state).
    runtime: Mutex<Runtime>,
}

/// The live runtime a daemon owns for the length of its own life.
#[derive(Default)]
pub struct Runtime {
    pub sessions: ManagedSessions,
    pub attach_tokens: AttachTokens,
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
            unreachable: AtomicBool::new(false),
            runtime: Mutex::new(Runtime::default()),
        })
    }

    /// Confirm the registry can still vouch for durable truth.
    ///
    /// What an answer derived from the registry needs before it may be given.
    /// Protocol 1 assigns no session encoding, so nothing this build serves
    /// carries a fact out of the store — but an empty list is still a claim
    /// about it, and this is the question behind that claim.
    /// Contention is the only refusal this call can produce, and the only one
    /// reported as retryable: `busy` tells a client to send the request again,
    /// and saying that about a refusal nothing diagnosed would be a claim the
    /// daemon cannot make. Anything else is returned as it is, for the caller
    /// to decide — and a refusal still never ends the daemon, because a
    /// refusal leaves the store intact.
    ///
    /// The mapping is this call's, not a shared one: a mutating method's
    /// refusals are mostly permanent, and the phase that serves one writes its
    /// own.
    pub async fn vouch(self: &Arc<Self>) -> Result<Vouched, StateError> {
        match self.off_the_reactor(Store::vouch).await {
            Ok(()) => Ok(Vouched::Yes),
            Err(StateError::Refused(Refusal::Busy { .. })) => Ok(Vouched::NotNow),
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
    /// Work with the live runtime.
    ///
    /// Synchronous and short: these calls touch in-memory state and message a
    /// session's own thread, so they never wait on a process the way a store
    /// call can wait on a database.
    pub fn with_runtime<T>(&self, work: impl FnOnce(&mut Runtime) -> T) -> Option<T> {
        // A poisoned lock means a holder panicked mid-mutation, which is not
        // something to paper over: the caller answers with what it says when
        // the runtime cannot be consulted rather than reading state nobody
        // finished writing.
        self.runtime
            .lock()
            .ok()
            .map(|mut runtime| work(&mut runtime))
    }

    pub fn stopped_vouching(&self) -> bool {
        self.unreachable.load(Ordering::SeqCst) || self.lock().stopped_vouching()
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
            // The store never saw this, so it is recorded here instead; an exit
            // status read from the store alone would report a clean stop.
            Err(source) => {
                self.unreachable.store(true, Ordering::SeqCst);
                Err(StateError::Fatal(FatalState::Storage {
                    detail: format!("a registry call did not complete: {source}"),
                }))
            }
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

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
