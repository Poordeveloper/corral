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

use std::sync::{Arc, Mutex};

use corral_state::{Durability, StateError, Store};
use tracing::{error, warn};

use crate::runtime::{ObservedRuns, RunOccurrence};

/// Record every run occurrence this daemon observes, on a thread of its own.
pub fn record_observed_runs(store: Arc<Mutex<Store>>, observed: ObservedRuns) {
    std::thread::spawn(move || {
        // Ends when the last runtime that could report is gone, which for this
        // daemon means the process is ending.
        while let Some(observation) = observed.next() {
            if !record(&store, observation.occurrence()) {
                observed.lost();
            }
        }
    });
}

/// Whether this fact reached the log, or is accounted for by not needing to.
fn record(store: &Mutex<Store>, occurrence: RunOccurrence) -> bool {
    // A poisoned lock means another holder panicked. The store decides for
    // itself whether it can still vouch, and refusing to look would stop every
    // later fact from ever being written.
    let mut store = store
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    match occurrence {
        RunOccurrence::Exited { run, end, at } => match store.record_run_ended(run, end, at) {
            Ok(Durability::Recorded) => true,
            // The log holds no live Run to close. For a managed run the start
            // barrier makes that impossible — `RunStarted` commits before the
            // threads that could report an end exist — so reaching this means
            // the barrier did not hold, and the store's answer was a silent
            // `Withheld` rather than an error (grill Q9).
            Ok(Durability::Withheld) => {
                warn!(%run, "a managed run ended with no durable start to close");
                false
            }
            Err(error) => {
                error!(%run, %error, "a managed run's ending could not be recorded");
                false
            }
        },
        RunOccurrence::Attached { run, at } => attachment(run, store.record_run_attached(run, at)),
        RunOccurrence::Detached { run, at } => attachment(run, store.record_run_detached(run, at)),
    }
}

/// An attachment fact, where the store withholding it is ordinary.
///
/// A person may attach to a finished session's screen, and may still be
/// attached when its run ends. Both leave an attachment fact about a closed
/// episode, and the log correctly records nothing: `RunEnded` is terminal for
/// that Run's attachment state, and a projection reads still-open attachments
/// as inactive after it rather than needing invented facts (grill Q11).
fn attachment(run: corral_core::RunId, outcome: Result<Durability, StateError>) -> bool {
    match outcome {
        Ok(_) => true,
        Err(error) => {
            error!(%run, %error, "an attachment fact could not be recorded");
            false
        }
    }
}
