//! Enumerate the providers' stores at start and on a cadence, resolve what
//! they hold against the Sessions Corral has, and keep the rest as rows.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use corral_core::ProviderId;
use tokio::sync::watch;
use tracing::{debug, warn};

use super::{HistoryEntry, Recent, enumerate, sealed_here, sealed_now, store_root};
use crate::provider::KnownProvider;
use crate::state::DaemonState;

/// How often the stores are read again. A session file changes when its
/// session acts, which the live rows already show; this cadence is for
/// sessions that exist only in the store.
pub const ENUMERATION_CADENCE: Duration = Duration::from_secs(30);

/// One pass over every sealed provider's store.
pub async fn enumerate_once(state: &Arc<DaemonState>, now: SystemTime) {
    pass(state, now, sealed_here).await;
}

/// One pass, told how to decide whether a provider's layout is sealed here.
///
/// The decision is a parameter because it is the whole of what makes a store
/// readable, and a test that cannot change it can only ever exercise this
/// machine's installation.
async fn pass(state: &Arc<DaemonState>, now: SystemTime, sealed: fn(KnownProvider) -> bool) {
    let Some(home) = state.provider_home() else {
        return;
    };
    for provider in KnownProvider::ALL {
        // Nothing is enumerated for a layout the matrix has not sealed at
        // the version installed here: a row is a claim that a session
        // exists, and the shape it is read from is what supports it (ADR
        // 0016 D1). An install Corral cannot version is unsealed for the
        // same reason — not knowing which layout is on disk is not a
        // licence to assume the measured one.
        if !sealed_now(provider, sealed).await {
            // Retracted, not merely skipped. What a previous pass learned was
            // supported by the layout sealed at the version installed then;
            // an install that is no longer sealed takes that support away, and
            // rows kept past it stay listable and continuable on evidence
            // this daemon can no longer stand behind (ADR 0016 D1).
            state.with_runtime(|runtime| runtime.history.retract(provider));
            continue;
        }
        let root = store_root(provider, &home);
        let entries = tokio::task::spawn_blocking(move || {
            enumerate(provider, &root, now, &Recent::default())
        })
        .await
        .unwrap_or_default();
        let Ok(named) = ProviderId::new(provider.as_str()) else {
            continue;
        };
        let mut unresolved: Vec<HistoryEntry> = Vec::new();
        let mut resolved = Vec::new();
        for entry in entries {
            match state
                .session_by_external_id(named.clone(), entry.external_id.clone())
                .await
            {
                Ok(Some(session)) => resolved.push((session, entry.last_active)),
                Ok(None) => unresolved.push(entry),
                Err(source) => {
                    // The store could not answer; the pass says nothing new
                    // rather than listing a Session Corral may already hold.
                    warn!(%source, "history enumeration could not consult the registry");
                    return;
                }
            }
        }
        debug!(
            provider = provider.as_str(),
            rows = unresolved.len(),
            known = resolved.len(),
            "history enumerated",
        );
        state.with_runtime(|runtime| runtime.history.replace(provider, unresolved, resolved));
    }
}

/// Enumerate until the daemon shuts down.
pub async fn enumerate_until_shutdown(
    state: Arc<DaemonState>,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        enumerate_once(&state, SystemTime::now()).await;
        tokio::select! {
            _ = shutdown.changed() => return,
            () = tokio::time::sleep(ENUMERATION_CADENCE) => {}
        }
    }
}

#[cfg(test)]
#[path = "task_tests.rs"]
mod tests;
