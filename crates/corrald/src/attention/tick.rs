//! The daemon's clock over the ledger: activity is read off every managed
//! screen, every Session is re-derived, and what changed is journaled.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use corral_core::{
    Assurance, Channel, Claim, CorralSessionId, EvidenceSource, MainState, Sealing, SemanticState,
};
use tokio::sync::watch;
use tracing::debug;

use super::{Change, Record, Transition, TransitionRecord};
use crate::runtime::ExecutionState;
use crate::state::DaemonState;

/// How often claims are re-judged against their horizons. A rot lands
/// within one of these of its horizon; nothing else waits on it.
pub const FRESHNESS_TICK: Duration = Duration::from_secs(1);

/// One tick: observe activity, derive every Session, journal the changes.
pub fn tick_once(state: &Arc<DaemonState>, now: SystemTime) -> Vec<Change> {
    let changes = state
        .with_runtime(|runtime| {
            let mut activity = Vec::new();
            for session in runtime.sessions.describe() {
                let Some(handle) = runtime.sessions.get(session.session) else {
                    continue;
                };
                let Some(drawn) = handle.last_output_at() else {
                    continue;
                };
                activity.push((session.session, drawn));
            }
            // Both sources are presented every tick, and the ledger keeps
            // only what is actually newer: a screen that still supports its
            // reading dates it forward and so stays the newest claim (ADR 0015
            // D4), while one that has not been read again presents the same
            // fact and changes nothing. An unsealed reading is presented too
            // and refused by entitlement, which is what makes it countable
            // without making it a claim.
            let mut readings = Vec::new();
            for session in runtime.sessions.describe() {
                let Some(handle) = runtime.sessions.get(session.session) else {
                    continue;
                };
                if let Some(reading) = handle.reading() {
                    readings.push((session.session, reading));
                }
            }
            for (session, reading) in readings {
                runtime.attention.observe(
                    session,
                    Claim {
                        source: EvidenceSource::ScreenDetection,
                        association: Assurance::Deterministic,
                        channel: Channel::CorralOwnedPty,
                        sealing: reading.sealing,
                        asserts: reading.asserts,
                    },
                    // Dated by the screen thread, not by this clock: the
                    // reading is only as current as the last moment the screen
                    // was known to support it, and a screen being redrawn
                    // faster than it settles supports nothing.
                    reading.at,
                );
            }
            for (session, drawn) in activity {
                runtime.attention.observe(
                    session,
                    Claim {
                        source: EvidenceSource::PtyActivity,
                        association: Assurance::Deterministic,
                        channel: Channel::CorralOwnedPty,
                        // Activity needs no matrix: bytes on a PTY Corral owns
                        // are the agent drawing, whatever version it is.
                        sealing: Sealing::Sealed,
                        asserts: SemanticState::Working,
                    },
                    drawn,
                );
            }
            let managed: std::collections::HashMap<CorralSessionId, ExecutionState> = runtime
                .sessions
                .describe()
                .into_iter()
                .map(|session| (session.session, session.execution_state))
                .collect();
            let external: std::collections::HashSet<CorralSessionId> = state
                .seen_runtimes()
                .snapshot()
                .iter()
                .filter_map(|candidate| candidate.identified().map(|identified| identified.session))
                .collect();
            let changes = runtime.attention.tick(now, |session| {
                managed
                    .get(&session)
                    .copied()
                    .unwrap_or(if external.contains(&session) {
                        ExecutionState::Running
                    } else {
                        ExecutionState::Unknown
                    })
            });
            // The version bound to the runtime that produced the evidence, as
            // the launch boundary established it — read here rather than
            // carried on every claim, and absent where this daemon never
            // bound one, which is unknown and not a version (grill Q12).
            changes
                .into_iter()
                .map(|change| {
                    let version = runtime
                        .reported
                        .get(change.session)
                        .and_then(|reported| reported.provider_version.clone());
                    (change, version)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !changes.is_empty() {
        state.journal_append(
            now,
            changes
                .iter()
                .map(|(change, version)| record(change, version.clone()))
                .collect(),
        );
    }
    changes.into_iter().map(|(change, _)| change).collect()
}

fn record(change: &Change, provider_version: Option<String>) -> Record {
    let (born, ended, item_end, notifiable) = match change.transition {
        Transition::ItemBorn(item) => (Some(item), None, None, true),
        Transition::ItemEnded { item, end } => (None, Some(item), Some(end), false),
        Transition::ItemReplaced { ended, end, born } => (Some(born), Some(ended), Some(end), true),
        Transition::StateChanged { .. } | Transition::Unchanged => (None, None, None, false),
    };
    Record::Transition(TransitionRecord {
        session: change.session,
        from: change.from,
        to: change.to,
        source: change.decided_by.map(|claim| claim.source),
        assurance: change.decided_by.map(|claim| claim.association),
        sealed: change
            .decided_by
            .map(|claim| claim.sealing == Sealing::Sealed),
        provider_version,
        horizon: change.horizon,
        expired_after: change.expired_after,
        // Which of the two ways a claim can stop standing happened here: a
        // horizon ran out, or something fresher said otherwise. An exit is
        // neither, and neither is a state no claim decided.
        contradicted_first: match (change.to, change.expired_after) {
            (_, Some(_)) => Some(false),
            (MainState::Exited, None) => None,
            (_, None) => change.decided_by.map(|_| true),
        },
        born,
        ended,
        item_end,
        notifiable,
    })
}

/// Tick until the daemon shuts down.
pub async fn tick_until_shutdown(state: Arc<DaemonState>, mut shutdown: watch::Receiver<bool>) {
    let mut interval = tokio::time::interval(FRESHNESS_TICK);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = interval.tick() => {}
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return;
                }
            }
        }
        let ticking = Arc::clone(&state);
        // Off the reactor: the ledger lock is brief, the journal write is a
        // file append, and neither belongs on the one thread every connection
        // shares.
        let changes = tokio::task::spawn_blocking(move || tick_once(&ticking, SystemTime::now()))
            .await
            .unwrap_or_default();
        for change in changes {
            debug!(session = %change.session, from = ?change.from, to = ?change.to, "attention changed");
        }
    }
}

#[cfg(test)]
#[path = "tick_tests.rs"]
mod tests;
