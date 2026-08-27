//! The bridge between what the runtime observed and what the store records.
//!
//! The split it keeps is the one the design froze: runtime owns occurrence
//! detection, state owns durable truth, and the bridge must never make runtime
//! teardown wait on database latency (grill Q6). So it is a thread of its own
//! behind a bounded queue, and nothing on a reaper's or a retiring screen's
//! path ever touches SQLite.
//!
//! It records rather than decides. Whether a fact may be written is the
//! store's question; what this owns is that every fact reaches it, and that a
//! fact which does not is loud.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use corral_core::RunEnd;
use corral_state::{Durability, Refusal, StateError, Store};
use tracing::{error, warn};

use crate::provider::InjectedSettings;
use crate::runtime::{ObservedRuns, RunOccurrence, Weight};

/// How many times one fact is offered to a store that is momentarily held.
///
/// Contention is the canonical transient condition — the store waits out its
/// own busy timeout before saying so, and its own rule is that concluding a
/// store is broken from contention would let one backup tool end the daemon.
/// So a `Busy` is waited out again rather than turned into integrity loss.
/// Bounded, because a fact that cannot be written after this is no longer a
/// wait: it is a hole in the accounting, and the daemon says so.
const ATTEMPTS: u32 = 3;

/// The pause between them. Short next to the store's own wait, which is what
/// actually does the waiting; this only avoids re-entering it on the same
/// instant.
const BETWEEN_ATTEMPTS_MILLIS: u64 = 50;
const BETWEEN_ATTEMPTS: Duration = Duration::from_millis(BETWEEN_ATTEMPTS_MILLIS);

/// What the store itself spends waiting for another writer before refusing.
///
/// Stated here because the budget below is a multiple of it. Not read from the
/// store: it is that crate's number to choose, and a copy that drifted would
/// only ever make this budget too generous, never too short.
const STORE_WAIT_MILLIS: u64 = 5_000;

/// The longest the recorder may spend on one fact before calling it lost.
///
/// Derived rather than chosen, and public because a shutdown has to outwait
/// it: settling for less would declare a hole in the accounting while a write
/// was still legitimately in progress, and two owners of one deadline is how
/// that happens.
pub const LONGEST_RECORD: Duration =
    Duration::from_millis(ATTEMPTS as u64 * (STORE_WAIT_MILLIS + BETWEEN_ATTEMPTS_MILLIS));

/// Record every run occurrence this daemon observes, on a thread of its own.
///
/// It also destroys the per-launch provider configuration of a Run whose exit
/// it just established. That belongs here rather than beside the endpoint
/// because this is the one place that learns an exit *and* what kind of end it
/// was: the file is removed on an established exit and retained on an
/// unverifiable one, and no other party has both facts (ADR 0004 D6).
pub fn record_observed_runs(store: Arc<Mutex<Store>>, observed: ObservedRuns, launch_dir: PathBuf) {
    std::thread::spawn(move || {
        // Ends when the last runtime that could report is gone, which for this
        // daemon means the process is ending.
        while let Some(observation) = observed.next() {
            let occurrence = observation.occurrence();
            if let RunOccurrence::Exited {
                run,
                end: RunEnd::Exited(_),
                ..
            } = occurrence
            {
                InjectedSettings::remove_for(&launch_dir, run);
            }
            // Only a fact the daemon must be able to account for ends the
            // accounting. An attachment the store would not take costs a line
            // of history and nothing else: attachment activity is advisory,
            // managed runtime ownership is authoritative (founder ruling,
            // 2026-08-25).
            if !record_within_attempts(&store, occurrence)
                && occurrence.weight() == Weight::Authoritative
            {
                observed.lost();
            }
        }
    });
}

/// Record one fact, waiting out a store another writer is holding.
fn record_within_attempts(store: &Mutex<Store>, occurrence: RunOccurrence) -> bool {
    for attempt in 1..=ATTEMPTS {
        match record(store, occurrence) {
            Recorded::Yes => return true,
            Recorded::NotNow => {
                if attempt < ATTEMPTS {
                    std::thread::sleep(BETWEEN_ATTEMPTS);
                }
            }
            Recorded::No => return false,
        }
    }
    false
}

/// What became of one fact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Recorded {
    /// In the log, or accounted for by not needing to be.
    Yes,
    /// The store was held by another writer. Nothing happened, and the same
    /// fact may be offered again.
    NotNow,
    /// This daemon can no longer account for its runs.
    No,
}

/// Whether this fact reached the log, or is accounted for by not needing to.
fn record(store: &Mutex<Store>, occurrence: RunOccurrence) -> Recorded {
    // A poisoned lock means another holder panicked. The store decides for
    // itself whether it can still vouch, and refusing to look would stop every
    // later fact from ever being written.
    let mut store = store
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    match occurrence {
        RunOccurrence::Exited { run, end, at } => match store.record_run_ended(run, end, at) {
            Ok(Durability::Recorded) => Recorded::Yes,
            // The log holds no live Run to close. For a managed run the start
            // barrier makes that impossible — `RunStarted` commits before the
            // threads that could report an end exist — so reaching this means
            // the barrier did not hold, and the store's answer was a silent
            // `Withheld` rather than an error (grill Q9).
            Ok(Durability::Withheld) => {
                warn!(%run, "a managed run ended with no durable start to close");
                Recorded::No
            }
            // The end is already recorded. That is the outcome this fact
            // wanted, not a hole in the accounting — and reading it as one
            // would end the daemon over a duplicate, which a commit that
            // landed and then reported contention produces for free.
            Err(StateError::Refused(Refusal::RunAlreadyEnded(_))) => Recorded::Yes,
            Err(error) => held_or_lost(run, &error, "a managed run's ending"),
        },
        RunOccurrence::Attached { run, at } => attachment(run, store.record_run_attached(run, at)),
        RunOccurrence::Detached { run, at } => attachment(run, store.record_run_detached(run, at)),
    }
}

/// An attachment fact, where the episode being closed is ordinary.
///
/// A person may attach to a finished session's screen, and may still be
/// attached when its run ends — `corral new -- true` is both. The store
/// refuses the fact, because a Run's outcome is stated once and the log is
/// never rewritten. That refusal is the answer, not a failure: `RunEnded` is
/// terminal for a Run's attachment state, and a projection reads still-open
/// attachments as inactive after it rather than needing the fact at all
/// (grill Q11).
///
/// Reading it as integrity loss would shut the daemon down every time somebody
/// was watching when an agent finished.
fn attachment(run: corral_core::RunId, outcome: Result<Durability, StateError>) -> Recorded {
    match outcome {
        Ok(Durability::Recorded) | Err(StateError::Refused(Refusal::RunAlreadyEnded(_))) => {
            Recorded::Yes
        }
        // Not the same answer as an ended episode, and the difference matters:
        // this one says the log has never heard of the Run. A handle to attach
        // to exists only after `RunStarted` committed, so reaching this means
        // the barrier did not hold — the same failure the ending arm reports,
        // and accepting it here would hide it behind the one fact this daemon
        // can afford to lose.
        // The log has never heard of the Run. A handle to attach to exists
        // only after `RunStarted` committed, so this means the barrier did not
        // hold — and it is said out loud here even though it costs no
        // integrity, because the ending path will reach the same conclusion on
        // a fact that does.
        Ok(Durability::Withheld) => {
            warn!(%run, "an attachment names a run with no durable start");
            Recorded::No
        }
        Err(error) => held_or_lost(run, &error, "an attachment fact"),
    }
}

/// Whether a store that refused is one to wait for or one to give up on.
///
/// Contention is the only transient refusal the store produces, and it is the
/// one the store itself says must never be read as a broken store. Everything
/// else leaves this daemon unable to account for its runs.
fn held_or_lost(run: corral_core::RunId, error: &StateError, what: &str) -> Recorded {
    if matches!(error, StateError::Refused(Refusal::Busy { .. })) {
        warn!(%run, "{what} is waiting for a registry another writer holds");
        return Recorded::NotNow;
    }
    error!(%run, %error, "{what} could not be recorded");
    Recorded::No
}
