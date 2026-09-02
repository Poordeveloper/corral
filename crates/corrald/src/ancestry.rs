//! Walking from where a relay stood to the provider process that ran it.
//!
//! The corroboration half of ADR 0014 D2. A token-less delivery says a
//! provider thread emitted an event; this says whether a supported provider
//! process was really there when it did. Together they are what promotes an
//! identity from a candidate to an attested binding — and either alone is
//! not.
//!
//! Daemon-side, always. The relay is forbidden this work: it is short-lived
//! and poor by contract, and a walk inside its budget would be interference
//! measured in the user's agent (ADR 0004 D4).
//!
//! Best-effort by nature. Hook processes are short-lived children and the
//! chain can be gone before it is read, so a failed walk degrades the claim
//! and never blocks ingestion.

use crate::platform::process::{self, Observation, ProcessIdentity};
use crate::provider::{KnownProvider, recognition};

/// How far up the chain a walk looks.
///
/// The measured chains are short: a Claude hook is two hops from its provider
/// — the process, the `/bin/sh -c` Claude runs it through, the provider —
/// and a Codex notify is one, spawned by the provider directly. The bound is
/// generous next to both and exists because a process tree is somebody else's
/// data structure: a cycle or a very deep chain must cost a bounded walk
/// rather than the daemon.
const MAX_HOPS: usize = 8;

/// What a walk concluded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Corroboration {
    /// A supported provider process was running, and here it is.
    Reached {
        provider: KnownProvider,
        process: Box<ProcessIdentity>,
    },
    /// The chain was readable and no supported provider was on it. Says the
    /// walk found nothing, never that nothing was there: the shapes above a
    /// provider are not sealed, and a chain that ran out is one of them.
    NotFound,
    /// The chain could not be read far enough to answer — the processes were
    /// gone, or this account may not inspect them. Unknown, and never
    /// collapsed into `NotFound`.
    Unreadable,
}

/// Walk up from a pid, looking for the provider a delivery claims.
///
/// The claimed provider is an input rather than an output: a delivery names
/// which provider's ingress it is, and a walk that returned whatever it found
/// would let a Codex notify corroborate itself against a Claude process that
/// happened to be an ancestor.
#[must_use]
pub fn corroborate(from: u32, claimed: KnownProvider) -> Corroboration {
    walk(from, claimed, &process::observe)
}

/// The walk itself, over an injectable observer.
///
/// Separated so the chain shapes — a provider one hop below its launcher, a
/// chain that ends, a cycle — are tested against process trees a test can
/// build, rather than against whatever this machine happens to be running.
fn walk(from: u32, claimed: KnownProvider, observe: &dyn Fn(u32) -> Observation) -> Corroboration {
    let mut current = from;
    let mut read_any_hop = false;
    let mut seen = Vec::with_capacity(MAX_HOPS);

    for _ in 0..MAX_HOPS {
        // A pid that repeats means the chain is not a chain. Stopping is the
        // only safe answer: a tree Corral did not build can say anything.
        if seen.contains(&current) {
            break;
        }
        seen.push(current);

        let identity = match observe(current) {
            Observation::Identified(identity) => identity,
            // A hop that cannot be read is not the end of the chain, but
            // there is nowhere to go from it: the parent is a field on the
            // record this account could not have.
            Observation::Gone | Observation::NotPermitted | Observation::Unobservable => {
                return if read_any_hop {
                    Corroboration::NotFound
                } else {
                    Corroboration::Unreadable
                };
            }
        };
        read_any_hop = true;

        if recognition::provider_of(&identity.executable) == Some(claimed) {
            return Corroboration::Reached {
                provider: claimed,
                process: identity,
            };
        }
        // Every unrecognized hop is walked through — a launcher, a shell, a
        // terminal — because the chain is what is being followed, not the
        // current hop's identity. The measured npm channel is why: its real
        // agent is the native child, one hop below the `node` wrapper.
        current = identity.parent;
    }
    Corroboration::NotFound
}

#[cfg(test)]
#[path = "ancestry_tests.rs"]
mod tests;
