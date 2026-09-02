//! Finding provider processes that never sent Corral anything.
//!
//! The other half of ADR 0014 D2's observation mechanism. The delivery path
//! sees a session the moment it acts; a session that has been idle since
//! before Corral started acts never, and without this it would stay invisible
//! for as long as the user leaves it alone — which is exactly the session a
//! person most needs reminding of.
//!
//! What a sweep may claim is narrower than what it finds, and the gap is the
//! point (grill Q5, Q6′). A recognized provider process supports one claim:
//! *a supported provider runtime appears to be running here*. It does not name
//! a provider session, because the process table holds no session id, and it
//! is not promoted by being seen again. Identity arrives on the delivery path
//! or not at all.
//!
//! So the sweep records a **runtime candidate** and nothing durable. It is
//! live state: a restart forgets it and rediscovers whatever is still there,
//! which is the honest answer for evidence that never earned durability
//! (ADR 0014 D5).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::SystemTime;

use corral_core::CorralSessionId;

use crate::platform::process::{self, Observation, ProcessIdentity};
use crate::provider::{KnownProvider, recognition};

/// A provider runtime the sweep found, and what it is entitled to claim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeCandidate {
    provider: KnownProvider,
    process: ProcessIdentity,
    /// The Corral identity this provisional row is shown under.
    ///
    /// Minted once per incarnation and kept for as long as the runtime is
    /// seen, so a row does not change identity under the user between passes.
    /// It is not a Session: nothing durable is written under it, and the
    /// provider-id-keyed record a delivery mints is the one that wins
    /// (`ARCHITECTURE.md` §1) — listing that record in this row's place is
    /// the surfacing follow-up the plan names.
    provisional_id: CorralSessionId,
}

impl RuntimeCandidate {
    #[must_use]
    pub fn provider(&self) -> KnownProvider {
        self.provider
    }

    #[must_use]
    pub fn process(&self) -> &ProcessIdentity {
        &self.process
    }

    #[must_use]
    pub fn provisional_id(&self) -> CorralSessionId {
        self.provisional_id
    }
}

/// What one pass over the process table concluded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pass {
    /// The table was read.
    Read {
        /// The provider runtimes on it.
        found: Vec<RuntimeCandidate>,
        /// Pids the listing named that could not be inspected: this account
        /// may not look at them, or this build cannot. Neither answer says
        /// the process stopped, so a runtime held under one of these pids is
        /// not retired by its absence from `found`.
        uninspected: HashSet<u32>,
    },
    /// The table could not be enumerated. Not an empty machine: reading this
    /// as "nothing is running" would retire every runtime the last pass found.
    Unavailable,
}

/// Read the process table once and recognize what is on it.
///
/// Blocking, and the caller's job to keep off the reactor: a pass is a series
/// of system calls per process, and a contended one would stall every
/// connection the daemon is serving.
///
/// A pass records no time of its own. The only time a runtime has that Corral
/// may state is the one the kernel reports, and a first-observed instant is
/// never written as a start time (ADR 0002 D6) — keeping one here would put
/// it within reach of a caller that needed a start time and had none.
#[must_use]
pub fn once() -> Pass {
    let Some(pids) = process::all_pids() else {
        return Pass::Unavailable;
    };
    let mut found = Vec::new();
    let mut uninspected = HashSet::new();
    for pid in pids {
        match process::observe(pid) {
            Observation::Identified(process) => {
                let Some(provider) = recognition::provider_of(&process.executable) else {
                    continue;
                };
                found.push(RuntimeCandidate {
                    provider,
                    process: *process,
                    provisional_id: CorralSessionId::mint(),
                });
            }
            // Gone between the listing and the look. Ordinary on a live
            // machine, and the one answer that positively says so.
            Observation::Gone => {}
            // Not a finding, and not the absence of one either: a process
            // this account may not look at is still a process.
            Observation::NotPermitted | Observation::Unobservable => {
                uninspected.insert(pid);
            }
        }
    }
    Pass::Read { found, uninspected }
}

/// The provider runtimes this daemon currently believes are running.
///
/// Keyed on the incarnation rather than the pid: a reused pid is a different
/// runtime, and a table keyed on the number alone would carry the old one's
/// first-seen time into the new one's row.
#[derive(Default)]
pub struct SeenRuntimes {
    held: HashMap<Incarnation, RuntimeCandidate>,
}

/// One process incarnation: the pid, and the start time that tells it apart
/// from the next process to hold that number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Incarnation {
    pid: u32,
    started: SystemTime,
}

impl Incarnation {
    #[must_use]
    pub fn of(process: &ProcessIdentity) -> Self {
        Self {
            pid: process.pid,
            started: process.started,
        }
    }
}

/// What changed between two passes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    /// Runtimes this pass saw for the first time.
    pub appeared: Vec<RuntimeCandidate>,
    /// Runtimes the last pass held and this one established are not there.
    ///
    /// Only ever produced by a pass that actually read the table, and never
    /// for a pid that pass could not inspect: "I could not look" is not
    /// evidence that anything stopped, whether it is the whole table or one
    /// process.
    pub gone: Vec<RuntimeCandidate>,
}

impl SeenRuntimes {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold one pass in, and say what changed.
    pub fn absorb(&mut self, pass: Pass) -> Changes {
        let Pass::Read { found, uninspected } = pass else {
            return Changes::default();
        };
        let mut still_here = HashMap::with_capacity(found.len());
        let mut appeared = Vec::new();
        for candidate in found {
            let key = Incarnation::of(&candidate.process);
            match self.held.remove(&key) {
                // Seen before: it keeps the moment this daemon first saw it,
                // because a runtime does not become newer by being looked at
                // again.
                Some(known) => {
                    still_here.insert(key, known);
                }
                None => {
                    appeared.push(candidate.clone());
                    still_here.insert(key, candidate);
                }
            }
        }
        // A runtime whose pid this pass could not inspect stays held: unknown
        // is not gone. It is retired when a pass sees its pid absent, or sees
        // the pid carrying something else.
        let mut gone = Vec::new();
        for (key, candidate) in self.held.drain() {
            if uninspected.contains(&key.pid) {
                still_here.insert(key, candidate);
            } else {
                gone.push(candidate);
            }
        }
        self.held = still_here;
        Changes { appeared, gone }
    }

    pub fn all(&self) -> impl Iterator<Item = &RuntimeCandidate> {
        self.held.values()
    }
}

/// The sweep's table as the daemon's owners share it.
#[derive(Clone, Default)]
pub struct SharedSeenRuntimes(Arc<Mutex<SeenRuntimes>>);

impl SharedSeenRuntimes {
    #[must_use]
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(SeenRuntimes::new())))
    }

    pub fn absorb(&self, pass: Pass) -> Changes {
        self.held().absorb(pass)
    }

    #[must_use]
    pub fn snapshot(&self) -> Vec<RuntimeCandidate> {
        self.held().all().cloned().collect()
    }

    fn held(&self) -> MutexGuard<'_, SeenRuntimes> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
#[path = "sweep_tests.rs"]
mod tests;

/// Sweep at daemon start and on a bounded cadence, for as long as the daemon
/// runs.
///
/// Every pass is blocking work moved off the reactor. A pass that cannot read
/// the table changes nothing, so a machine that briefly refuses the listing
/// costs one quiet cycle rather than every runtime Corral had found.
pub async fn sweep_until_shutdown(
    seen: SharedSeenRuntimes,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    loop {
        let pass = match tokio::task::spawn_blocking(once).await {
            Ok(pass) => pass,
            // The blocking pool is gone or the task panicked. Neither says
            // anything about what is running, so neither retires a runtime.
            Err(_) => Pass::Unavailable,
        };
        let changes = seen.absorb(pass);
        for candidate in &changes.appeared {
            tracing::info!(
                provider = candidate.provider().as_str(),
                pid = candidate.process().pid,
                "a provider runtime outside Corral appeared",
            );
        }
        for candidate in &changes.gone {
            tracing::info!(
                provider = candidate.provider().as_str(),
                pid = candidate.process().pid,
                "a provider runtime outside Corral is no longer running",
            );
        }
        tokio::select! {
            _ = shutdown.changed() => return,
            () = tokio::time::sleep(crate::policy::SWEEP_CADENCE) => {}
        }
    }
}
