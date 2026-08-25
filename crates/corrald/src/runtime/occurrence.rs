//! What the managed runtime observed about a Run, and where it says so.
//!
//! Runtime owns occurrence detection; whoever drains the other end owns what
//! becomes of a fact. Nothing here knows that durable state exists, which is
//! the point: the reaper and the retiring screen must never wait on a database
//! to finish tearing a session down (grill Q6).
//!
//! Three rules make that safe rather than merely fast. Reporting never blocks.
//! A fact the daemon must account for is never silently dropped — a queue that
//! cannot take one means the run lifecycle has a hole in it, which is an
//! integrity failure and not backpressure. And what has been reported but not
//! yet recorded is countable, so a shutdown can wait for it instead of
//! discovering afterwards that it did not (grill Q10).
//!
//! What the daemon must account for is not everything it observes:
//!
//! > Attachment activity is advisory.
//! > Managed runtime ownership is authoritative.
//!
//! An observer attaching and detaching says nothing about who owns a runtime,
//! so it may inform diagnostics and never lifecycle truth. `Weight` is where
//! that line is drawn, and it is why churn cannot reach the daemon's own
//! lifetime (founder ruling, 2026-08-25).

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime};

use corral_core::{OccurrenceTime, RunEnd, RunId};
use tracing::debug;

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

impl RunOccurrence {
    /// What losing this one costs.
    #[must_use]
    pub fn weight(self) -> Weight {
        match self {
            Self::Exited { .. } => Weight::Authoritative,
            Self::Attached { .. } | Self::Detached { .. } => Weight::Advisory,
        }
    }

    /// The Run this is about, for whoever reports on it.
    #[must_use]
    pub fn run(self) -> RunId {
        match self {
            Self::Exited { run, .. } | Self::Attached { run, .. } | Self::Detached { run, .. } => {
                run
            }
        }
    }
}

/// What a lost observation costs.
///
/// **Attachment activity is advisory. Managed runtime ownership is
/// authoritative.** Attaching is something an observer does; it is not a claim
/// on the runtime, so it may inform diagnostics, buffer cleanup and a UI hint,
/// and it may never change lifecycle truth (founder ruling, 2026-08-25).
///
/// The practical consequence is that these two must not share a fate. An
/// ending nobody recorded leaves a durable Run that looks legitimate and stays
/// open forever; an attachment nobody recorded costs a line of history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weight {
    /// The daemon cannot account for its runs without it.
    Authoritative,
    /// Diagnostics and nothing else.
    Advisory,
}

/// How many observations may wait to be recorded.
///
/// An initial implementation value, not canon: run endings are rare and one
/// local transaction is sub-millisecond, so this is sized against attach churn
/// rather than against steady lifecycle load (grill Q10). Deep enough that
/// ordinary use never approaches it; small enough that exhaustion means
/// something is wrong rather than merely busy.
pub const OBSERVATION_QUEUE: usize = 1024;

/// How much of that queue advisory activity may occupy.
///
/// Bounded well below the whole, so churn exhausts its own budget and never
/// the room an ending needs. Without this the split above would be words: a
/// client connecting and disconnecting in a loop could fill the queue with
/// attachment facts and leave a run's ending with nowhere to go, which is the
/// daemon's whole control plane lost to an observer's behaviour.
///
/// It is the shape a slow viewer already has, one layer down: overflow costs
/// the subscription, never the session (`stream.rs`).
pub const ADVISORY_SHARE: usize = OBSERVATION_QUEUE / 4;

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
    in_flight: Arc<InFlight>,
    integrity: Arc<tokio::sync::watch::Sender<Integrity>>,
}

/// The draining end, held by whoever turns observations into durable facts.
pub struct ObservedRuns {
    observed: std::sync::mpsc::Receiver<RunOccurrence>,
    in_flight: Arc<InFlight>,
    integrity: Arc<tokio::sync::watch::Sender<Integrity>>,
}

/// How many observations have been reported and not yet recorded.
///
/// Counted by weight, because the two are spent against different budgets and
/// only one of them is worth waiting for on the way out.
///
/// A count with a way to wait on it, rather than a bare atomic: a shutdown
/// that wanted to know when the last fact had landed would otherwise have to
/// poll, and a poll is a guess with a sleep in it.
struct InFlight {
    counts: Mutex<Counts>,
    settled: Condvar,
}

#[derive(Default)]
struct Counts {
    authoritative: usize,
    advisory: usize,
}

/// One observation on its way into durable state.
///
/// It counts as outstanding until this is dropped, which is after whoever
/// recorded it has finished — so "nothing outstanding" means every reported
/// fact has been acted on, not merely dequeued.
pub struct Observed<'a> {
    occurrence: RunOccurrence,
    in_flight: &'a InFlight,
}

/// Open the channel between the runtime and whoever records what it sees.
#[must_use]
pub fn observe_runs() -> (RunObservations, ObservedRuns) {
    let (sender, receiver) = std::sync::mpsc::sync_channel(OBSERVATION_QUEUE);
    let in_flight = Arc::new(InFlight {
        counts: Mutex::new(Counts::default()),
        settled: Condvar::new(),
    });
    let (integrity, _) = tokio::sync::watch::channel(Integrity::Intact);
    let integrity = Arc::new(integrity);
    (
        RunObservations {
            observed: sender,
            in_flight: Arc::clone(&in_flight),
            integrity: Arc::clone(&integrity),
        },
        ObservedRuns {
            observed: receiver,
            in_flight,
            integrity,
        },
    )
}

impl RunObservations {
    /// Report what this runtime saw. Never waits.
    ///
    /// What a report that cannot be taken costs depends on what it is. An
    /// ending the daemon could not hand on ends its ability to account for its
    /// runs, and says so — dropping one quietly would leave a durable Run that
    /// looks legitimate and stays open forever, the most dangerous shape this
    /// design can produce (grill Q9, Q10).
    ///
    /// An attachment is an observer's activity, not a claim on the runtime. It
    /// is spent against a budget of its own, and exhausting that budget costs
    /// the fact and nothing else: a client connecting and disconnecting in a
    /// loop must not be able to reach the daemon's lifecycle (founder ruling,
    /// 2026-08-25).
    pub fn report(&self, occurrence: RunOccurrence) {
        let weight = occurrence.weight();
        if !self.in_flight.admit(weight) {
            debug!(
                run = %occurrence.run(),
                "attachment activity is outrunning the recorder; this one is not kept"
            );
            return;
        }
        if self.observed.try_send(occurrence).is_err() {
            self.in_flight.recorded(weight);
            match weight {
                Weight::Authoritative => self.lost(),
                Weight::Advisory => debug!(
                    run = %occurrence.run(),
                    "an attachment fact found no room and is not kept"
                ),
            }
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
        // Only the authoritative facts. An attachment still queued at exit is
        // a line of history nobody will write, which is what "advisory" means.
        if !self.in_flight.wait_until_accounted(within) {
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
            in_flight: &self.in_flight,
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
        self.in_flight.recorded(self.occurrence.weight());
    }
}

impl InFlight {
    /// Take a place in the queue, or refuse one that is not this weight's to
    /// take.
    ///
    /// Only advisory activity can be refused here. An authoritative fact is
    /// always admitted, because the queue's whole remaining depth is reserved
    /// for it — and if even that is full, the caller loses the accounting
    /// rather than the fact being dropped quietly.
    fn admit(&self, weight: Weight) -> bool {
        let mut counts = self.lock();
        match weight {
            Weight::Authoritative => counts.authoritative += 1,
            Weight::Advisory if counts.advisory >= ADVISORY_SHARE => return false,
            Weight::Advisory => counts.advisory += 1,
        }
        true
    }

    fn recorded(&self, weight: Weight) {
        let mut counts = self.lock();
        match weight {
            Weight::Authoritative => {
                counts.authoritative = counts.authoritative.saturating_sub(1);
                if counts.authoritative == 0 {
                    self.settled.notify_all();
                }
            }
            Weight::Advisory => counts.advisory = counts.advisory.saturating_sub(1),
        }
    }

    /// Whether every fact the daemon must account for was recorded before the
    /// deadline.
    fn wait_until_accounted(&self, within: Duration) -> bool {
        let counts = self.lock();
        let (counts, timed_out) = self
            .settled
            .wait_timeout_while(counts, within, |counts| counts.authoritative > 0)
            // A poisoned lock means a recorder panicked mid-count. The numbers
            // it guards are `usize`; refusing to look would turn one panic
            // into a shutdown that waits for a wakeup nobody will send.
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        !timed_out.timed_out() && counts.authoritative == 0
    }

    /// A poisoned lock leaves a count that may be one too high; that costs a
    /// shutdown its grace, where refusing to look would cost it its exit.
    fn lock(&self) -> std::sync::MutexGuard<'_, Counts> {
        self.counts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
#[path = "occurrence_tests.rs"]
mod tests;
