//! The daemon's clock over the ledger: activity is read off every managed
//! screen, every Session is re-derived, and what changed is journaled.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use corral_core::{
    Assurance, Channel, Claim, CorralSessionId, EvidenceSource, Sealing, SemanticState,
};
use tokio::sync::watch;
use tracing::debug;

use super::{Change, ItemEnd, Record, Transition, TransitionRecord};
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
                let already = runtime.attention.last_activity(session.session);
                if already.is_none_or(|at| drawn > at) {
                    activity.push((session.session, drawn));
                }
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
            runtime.attention.tick(now, |session| {
                managed
                    .get(&session)
                    .copied()
                    .unwrap_or(if external.contains(&session) {
                        ExecutionState::Running
                    } else {
                        ExecutionState::Unknown
                    })
            })
        })
        .unwrap_or_default();
    if !changes.is_empty() {
        state.journal_append(now, changes.iter().map(record).collect());
    }
    changes
}

fn record(change: &Change) -> Record {
    let (item, item_end, notifiable) = match change.transition {
        Transition::ItemBorn(item) => (Some(item), None, true),
        Transition::ItemEnded { item, end } => (Some(item), Some(end), false),
        Transition::StateChanged { .. } | Transition::Unchanged => (None, None, false),
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
        provider_version: None,
        horizon: None,
        expired_after: None,
        contradicted_first: match item_end {
            Some(ItemEnd::Resolved) => Some(true),
            Some(ItemEnd::Rotted) => Some(false),
            Some(ItemEnd::Exited) | None => None,
        },
        item,
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
