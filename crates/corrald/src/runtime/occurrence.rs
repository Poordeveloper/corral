//! What the managed runtime observed about a Run, and where it says so.
//!
//! Runtime owns occurrence detection; whoever drains the other end owns what
//! becomes of a fact. Nothing here knows that durable state exists, which is
//! the point: the reaper and the retiring screen must never wait on a database
//! to finish tearing a session down (grill Q6).
//!
//! Three rules make that safe rather than merely fast. Reporting never blocks.
//! A fact is never silently dropped — a queue that cannot take one means the
//! daemon can no longer account for its own run lifecycle, which is an
//! integrity failure and not backpressure. And what has been reported but not
//! yet recorded is countable, so a shutdown can wait for it instead of
//! discovering afterwards that it did not (grill Q10).

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime};

use corral_core::{OccurrenceTime, RunEnd, RunId};

/// One thing the runtime saw happen to a Run.
///
/// The vocabulary is ADR 0002's, mapped here from platform detail — exit
/// codes, signal numbers — so none of it reaches the domain (`runtime/mod.rs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunOccurrence {
    /// The runtime ended, as the one party that reaps it established.
    Exited {
        run: RunId,
        end: RunEnd,
        at: OccurrenceTime,
    },
    /// An established Corral attachment became active.
    ///
    /// Advisory: it carries no holder, no client identity, and no ownership
    /// claim, because PR3 has none of those to be honest about (grill Q7).
    Attached { run: RunId, at: SystemTime },
    /// An attachment ended while the daemon could observe it. Never the end of
    /// the Run — closing a surface does not terminate managed work.
    Detached { run: RunId, at: SystemTime },
}

/// How many observations may wait to be recorded.
///
/// An initial implementation value, not canon: run endings are rare and one
/// local transaction is sub-millisecond, so this is sized against attach churn
/// rather than against steady lifecycle load (grill Q10). Deep enough that
/// ordinary use never approaches it; small enough that exhaustion means
/// something is wrong rather than merely busy.
pub const OBSERVATION_QUEUE: usize = 1024;

/// Whether every observed occurrence reached whoever records it.
///
/// `Lost` is terminal. Nothing re-establishes an accounting that has a hole in
/// it, and the daemon's answer to it is to stop rather than to serve from a
/// durable state it can no longer vouch for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Integrity {
    Intact,
    Lost,
}

/// The reporting end, held by every thread that watches a runtime.
#[derive(Clone)]
pub struct RunObservations {
    observed: std::sync::mpsc::SyncSender<RunOccurrence>,
    outstanding: Arc<Outstanding>,
    integrity: Arc<tokio::sync::watch::Sender<Integrity>>,
}

/// The draining end, held by whoever turns observations into durable facts.
pub struct ObservedRuns {
    observed: std::sync::mpsc::Receiver<RunOccurrence>,
    outstanding: Arc<Outstanding>,
    integrity: Arc<tokio::sync::watch::Sender<Integrity>>,
}

/// How many observations have been reported and not yet recorded.
///
/// A count with a way to wait on it, rather than a bare atomic: a shutdown
/// that wanted to know when the last fact had landed would otherwise have to
/// poll, and a poll is a guess with a sleep in it.
struct Outstanding {
    count: Mutex<usize>,
    emptied: Condvar,
}

/// One observation on its way into durable state.
///
/// It counts as outstanding until this is dropped, which is after whoever
/// recorded it has finished — so "nothing outstanding" means every reported
/// fact has been acted on, not merely dequeued.
pub struct Observed<'a> {
    occurrence: RunOccurrence,
    outstanding: &'a Outstanding,
}

/// Open the channel between the runtime and whoever records what it sees.
#[must_use]
pub fn observe_runs() -> (RunObservations, ObservedRuns) {
    let (sender, receiver) = std::sync::mpsc::sync_channel(OBSERVATION_QUEUE);
    let outstanding = Arc::new(Outstanding {
        count: Mutex::new(0),
        emptied: Condvar::new(),
    });
    let (integrity, _) = tokio::sync::watch::channel(Integrity::Intact);
    let integrity = Arc::new(integrity);
    (
        RunObservations {
            observed: sender,
            outstanding: Arc::clone(&outstanding),
            integrity: Arc::clone(&integrity),
        },
        ObservedRuns {
            observed: receiver,
            outstanding,
            integrity,
        },
    )
}

impl RunObservations {
    /// Report what this runtime saw. Never waits.
    ///
    /// A full queue or a recorder that is gone ends the daemon's ability to
    /// account for its runs, and says so. It does not drop the fact quietly
    /// and carry on: an unrecorded ending leaves a durable Run that looks
    /// legitimate and stays open forever, which is the most dangerous shape
    /// this design can produce (grill Q9, Q10).
    pub fn report(&self, occurrence: RunOccurrence) {
        self.outstanding.enter();
        if self.observed.try_send(occurrence).is_err() {
            self.outstanding.leave();
            self.lost();
        }
    }

    /// Give up on accounting for this daemon's runs.
    pub fn lost(&self) {
        self.integrity.send_replace(Integrity::Lost);
    }

    pub fn integrity(&self) -> Integrity {
        *self.integrity.borrow()
    }

    /// Wake whoever is waiting when integrity is lost.
    pub fn watch_integrity(&self) -> tokio::sync::watch::Receiver<Integrity> {
        self.integrity.subscribe()
    }

    /// Wait for every reported observation to be recorded.
    ///
    /// For a daemon on its way out: facts still in the queue when the process
    /// ends are facts nobody will ever write, and leaving without asking is
    /// exactly the silent loss this channel exists to prevent. A wait that
    /// runs out is itself the answer — integrity is lost, and the exit status
    /// says so.
    ///
    /// It settles what was already reported; it does not close the channel.
    /// A managed run whose child dies in the window between this returning and
    /// the process exiting reports an ending nothing will drain, and the
    /// backstop for that is the next daemon's reconciliation, which closes the
    /// episode as unverifiable — which is what Corral can honestly say about a
    /// run whose end it did not record (ADR 0007 L6, grill Q5). Refusing to
    /// exit until such a run had reported would make a shutdown wait on the
    /// children ADR 0007 L6 says Corral does not wait for.
    pub fn settle(&self, within: Duration) -> Integrity {
        if !self.outstanding.wait_until_empty(within) {
            self.lost();
        }
        self.integrity()
    }
}

impl ObservedRuns {
    /// The next observation, or `None` when no runtime can report again.
    pub fn next(&self) -> Option<Observed<'_>> {
        self.observed.recv().ok().map(|occurrence| Observed {
            occurrence,
            outstanding: &self.outstanding,
        })
    }

    /// A fact could not be recorded. The daemon can no longer account for its
    /// runs, whatever the store itself still says.
    pub fn lost(&self) {
        self.integrity.send_replace(Integrity::Lost);
    }
}

impl Observed<'_> {
    pub fn occurrence(&self) -> RunOccurrence {
        self.occurrence
    }
}

impl Drop for Observed<'_> {
    fn drop(&mut self) {
        self.outstanding.leave();
    }
}

impl Outstanding {
    fn enter(&self) {
        *self.lock() += 1;
    }

    fn leave(&self) {
        let mut count = self.lock();
        *count = count.saturating_sub(1);
        if *count == 0 {
            self.emptied.notify_all();
        }
    }

    /// Whether everything reported was recorded before the deadline.
    fn wait_until_empty(&self, within: Duration) -> bool {
        let count = self.lock();
        let (count, timed_out) = self
            .emptied
            .wait_timeout_while(count, within, |count| *count > 0)
            // A poisoned lock means a recorder panicked mid-count. The number
            // it guards is a `usize`; refusing to look would turn one panic
            // into a shutdown that waits for a wakeup nobody will send.
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        !timed_out.timed_out() && *count == 0
    }

    /// A poisoned lock leaves a count that may be one too high; that costs a
    /// shutdown its grace, where refusing to look would cost it its exit.
    fn lock(&self) -> std::sync::MutexGuard<'_, usize> {
        self.count
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
#[path = "occurrence_tests.rs"]
mod tests;
