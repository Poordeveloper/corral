use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use corral_core::{Command, OccurrenceTime, RunId};
use corral_state::{FatalState, Refusal, StartedManagedSession, StateError, Store};

use crate::in_flight::InFlightCommands;
use crate::runtime::{AttachTokens, Integrity, ManagedSessions, RunObservations, observe_runs};

/// How long a departing daemon waits for its last observed facts to land.
///
/// Derived from the recorder's own budget rather than chosen beside it. The
/// recorder legitimately waits out a store another writer is holding, and a
/// shutdown that gave up first would declare a hole in the accounting — and
/// exit non-zero — while the write was still going to succeed.
const SETTLE_GRACE: Duration = Duration::from_millis(
    crate::run_lifecycle::LONGEST_RECORD.as_millis() as u64 + STORE_WAIT_OVERSHOOT_MILLIS,
);

/// The recorder's budget bounds when it stops *starting* attempts; the attempt
/// under way when it runs out still has the store's own wait to spend.
const STORE_WAIT_OVERSHOOT_MILLIS: u64 = 5_000;

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
    /// Behind an `Arc` because the store has a second owner: the thread that
    /// records what the runtime observed. It is the same store — one log, one
    /// writer — reached without going through this type, so a session's
    /// teardown never waits on anything a connection is doing.
    store: Arc<Mutex<Store>>,

    /// Set when this daemon concluded it can no longer vouch for durable truth
    /// by a route the store itself never saw: a call that did not complete.
    /// The store latches its own conclusions; the exit status reads both.
    cannot_vouch: AtomicBool,

    /// Where the runtime reports what it saw, and the accounting that says
    /// whether all of it was recorded.
    observations: RunObservations,

    /// The mutating commands this daemon is executing right now.
    commands: InFlightCommands,
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
        let store = Arc::new(Mutex::new(Store::open(registry)?));
        // Started with the store, not with the server: a runtime that could
        // report an ending before anything was draining the channel would fill
        // it and lose the accounting the daemon exists to keep.
        let (observations, observed) = observe_runs();
        crate::run_lifecycle::record_observed_runs(Arc::clone(&store), observed);
        Ok(Self {
            store,
            cannot_vouch: AtomicBool::new(false),
            observations,
            commands: InFlightCommands::new(),
            runtime: Mutex::new(Runtime::default()),
        })
    }

    /// Where a managed runtime reports what it observed about its Run.
    pub fn observations(&self) -> &RunObservations {
        &self.observations
    }

    pub fn commands(&self) -> &InFlightCommands {
        &self.commands
    }

    /// Close every managed-runtime episode a departed daemon left open.
    ///
    /// Synchronous and before the endpoint is bound: reconciliation is part of
    /// deciding what this daemon's durable state says, and a client that
    /// connected first could be told about a Run that was about to be closed
    /// behind it (grill Q5).
    pub fn reconcile_managed_runs(&self) -> Result<Vec<RunId>, StateError> {
        self.lock().end_unowned_managed_runs()
    }

    /// Wait for every observed fact to be recorded, on the way out.
    pub fn settle_observations(&self) -> Integrity {
        self.observations.settle(SETTLE_GRACE)
    }

    /// What this command already did, if it has run before.
    pub async fn completed_managed_session(
        self: &Arc<Self>,
        command: Command,
    ) -> Result<Option<StartedManagedSession>, StateError> {
        self.off_the_reactor(move |store| store.completed_managed_session(&command))
            .await
    }

    /// Record a Session, its managed runtime binding, and its first Run.
    pub async fn start_managed_session(
        self: &Arc<Self>,
        command: Command,
        run: RunId,
        started: OccurrenceTime,
        at: SystemTime,
    ) -> Result<StartedManagedSession, StateError> {
        self.off_the_reactor(move |store| store.start_managed_session(&command, run, started, at))
            .await
    }

    /// Confirm the registry can still vouch for durable truth.
    ///
    /// What an answer derived from the registry needs before it may be given.
    /// A session list is answered from the runtime rather than the store, but
    /// it is still a claim made in the store's name — and a mutation must
    /// never be admitted under the condition a read is refused.
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

    /// How many managed runs are still running.
    ///
    /// Answered rather than announced: the idle check asks at the moment it
    /// decides, so no caller can forget to report a change and leave a daemon
    /// exiting under live work. Zero when the registry cannot be read, which
    /// only delays an exit rather than causing one.
    pub fn live_sessions(&self) -> usize {
        self.runtime
            .lock()
            .map(|runtime| runtime.sessions.live())
            .unwrap_or(0)
    }

    /// The managed runs this daemon still believes are running.
    ///
    /// For shutdown, which has to be able to name what it is about to end
    /// rather than count it (ADR 0007 L6). Empty when the runtime cannot be
    /// consulted: a shutdown does not stall on a lock, and silence is the
    /// honest report when nothing can be read.
    pub fn running_sessions(&self) -> Vec<crate::runtime::ManagedSession> {
        self.with_runtime(|runtime| {
            runtime
                .sessions
                .describe()
                .into_iter()
                .filter(|session| {
                    session.execution_state == crate::runtime::ExecutionState::Running
                })
                .collect()
        })
        .unwrap_or_default()
    }

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
        let mut runtime = self.runtime.lock().ok()?;
        Some(work(&mut runtime))
    }

    /// Whether the registry has concluded it can no longer vouch for durable
    /// truth.
    ///
    /// Read from the store itself rather than from anything a connection task
    /// recorded: the conclusion has to survive the task that reached it being
    /// dropped mid-shutdown, or a daemon that stopped over an untrusted store
    /// could still exit as though nothing happened.
    pub fn stopped_vouching(&self) -> bool {
        self.cannot_vouch.load(Ordering::SeqCst)
            // A store that is perfectly healthy and a run lifecycle with a
            // hole in it are the same answer to the only question an exit
            // status can carry: this daemon could not keep its durable state
            // honest (grill Q10).
            || self.observations.integrity() == Integrity::Lost
            || self.lock().stopped_vouching()
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
                self.cannot_vouch.store(true, Ordering::SeqCst);
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
