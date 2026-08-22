use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tokio::sync::{Notify, watch};

/// The daemon's whole lifecycle. There is no path back from `ShuttingDown`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Running,
    ShuttingDown,
    Exited,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShutdownReason {
    /// No established client for the whole idle grace.
    Idle,
    Signal(&'static str),
    /// The registry store can no longer vouch for durable truth. The daemon
    /// stops serving rather than answering from an untrusted store, and exits
    /// non-zero so the next activation retries initialization (ADR 0002, Q14).
    FatalState,
}

/// What the idle watchdog should do next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdleCheck {
    /// This call took the Running → ShuttingDown transition.
    Committed,
    /// Someone else already committed.
    AlreadyCommitted,
    /// Established clients exist; only a change can make the daemon idle.
    Busy,
    /// Idle, with this much of the grace left.
    Wait(Duration),
}

/// How the daemon stopped, as far as its exit status is concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitDisposition {
    Clean,
    /// The daemon stopped serving because it could not trust durable state.
    ///
    /// Recorded separately from the shutdown reason: whichever cause commits
    /// the shutdown first wins the reason, and a signal arriving in the same
    /// moment must not turn an untrusted store into a clean exit.
    UntrustedState,
}

struct State {
    phase: Phase,
    established: usize,
    /// `Some` exactly while there are no established clients.
    idle_since: Option<Instant>,
    reason: Option<ShutdownReason>,
    untrusted_state: bool,
}

/// The single serialization point for "may this daemon exit".
///
/// Establishing a client and committing to shutdown are decided under the same
/// lock, so the two can never interleave into a daemon that exits while a
/// client believes it is connected. Once committed, shutdown is never
/// cancelled: a raw connection arriving afterwards has no power to revive it
/// (ADR 0001 D6).
pub struct Lifecycle {
    state: Mutex<State>,
    changed: Notify,
    shutdown: watch::Sender<bool>,
}

/// Proof that a connection completed the handshake.
///
/// Only established clients count towards daemon lifetime; dropping the guard
/// gives the countdown back.
pub struct EstablishedGuard {
    lifecycle: Arc<Lifecycle>,
}

impl Lifecycle {
    pub fn new(now: Instant) -> Arc<Self> {
        let (shutdown, _) = watch::channel(false);
        Arc::new(Self {
            state: Mutex::new(State {
                phase: Phase::Running,
                established: 0,
                idle_since: Some(now),
                reason: None,
                untrusted_state: false,
            }),
            changed: Notify::new(),
            shutdown,
        })
    }

    pub fn subscribe(&self) -> watch::Receiver<bool> {
        self.shutdown.subscribe()
    }

    pub fn changed(&self) -> &Notify {
        &self.changed
    }

    pub fn phase(&self) -> Phase {
        self.lock().phase
    }

    pub fn shutdown_reason(&self) -> Option<ShutdownReason> {
        self.lock().reason
    }

    pub fn established_clients(&self) -> usize {
        self.lock().established
    }

    /// Record that durable state can no longer be trusted. Never cleared.
    pub fn note_untrusted_state(&self) {
        self.lock().untrusted_state = true;
    }

    pub fn exit_disposition(&self) -> ExitDisposition {
        if self.lock().untrusted_state {
            ExitDisposition::UntrustedState
        } else {
            ExitDisposition::Clean
        }
    }

    /// Promote a handshaken connection to an established client.
    ///
    /// `None` means shutdown was already committed, so the connection must be
    /// closed rather than served.
    pub fn establish(self: &Arc<Self>) -> Option<EstablishedGuard> {
        {
            let mut state = self.lock();
            if state.phase != Phase::Running {
                return None;
            }
            state.established += 1;
            state.idle_since = None;
        }
        self.changed.notify_one();
        Some(EstablishedGuard {
            lifecycle: Arc::clone(self),
        })
    }

    /// Check idle eligibility and, if it holds, commit — one atomic step.
    pub fn poll_idle(&self, grace: Duration, now: Instant) -> IdleCheck {
        let committed = {
            let mut state = self.lock();
            if state.phase != Phase::Running {
                return IdleCheck::AlreadyCommitted;
            }
            match state.idle_since {
                None => return IdleCheck::Busy,
                Some(since) => {
                    let idle_for = now.saturating_duration_since(since);
                    match grace.checked_sub(idle_for) {
                        Some(remaining) if !remaining.is_zero() => {
                            return IdleCheck::Wait(remaining);
                        }
                        _ => {
                            state.phase = Phase::ShuttingDown;
                            state.reason = Some(ShutdownReason::Idle);
                            true
                        }
                    }
                }
            }
        };
        if committed {
            let _ = self.shutdown.send(true);
            return IdleCheck::Committed;
        }
        IdleCheck::AlreadyCommitted
    }

    /// Commit shutdown regardless of idle eligibility.
    ///
    /// Signals take the same committed path; the only difference is that they
    /// skip the eligibility question entirely — no grace, no waiting for
    /// in-flight requests.
    pub fn commit_shutdown(&self, reason: ShutdownReason) -> bool {
        {
            let mut state = self.lock();
            if state.phase != Phase::Running {
                return false;
            }
            state.phase = Phase::ShuttingDown;
            state.reason = Some(reason);
        }
        let _ = self.shutdown.send(true);
        true
    }

    pub fn mark_exited(&self) {
        self.lock().phase = Phase::Exited;
    }

    fn release(&self) {
        {
            let mut state = self.lock();
            state.established = state.established.saturating_sub(1);
            if state.established == 0 && state.idle_since.is_none() {
                state.idle_since = Some(Instant::now());
            }
        }
        self.changed.notify_one();
    }

    /// A poisoned lock means another thread panicked while holding it; the
    /// lifecycle counters are still coherent, and refusing to look at them
    /// would turn one panic into a stuck daemon.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Drop for EstablishedGuard {
    fn drop(&mut self) {
        self.lifecycle.release();
    }
}

/// Exit the daemon once it has been idle for the whole grace.
pub async fn watch_idle(lifecycle: Arc<Lifecycle>, grace: Duration) {
    loop {
        match lifecycle.poll_idle(grace, Instant::now()) {
            IdleCheck::Committed | IdleCheck::AlreadyCommitted => return,
            IdleCheck::Busy => lifecycle.changed().notified().await,
            IdleCheck::Wait(remaining) => {
                tokio::select! {
                    () = tokio::time::sleep(remaining) => {}
                    () = lifecycle.changed().notified() => {}
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
