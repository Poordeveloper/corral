use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use super::*;

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn registry(name: &str) -> (Arc<DaemonState>, PathBuf) {
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "corrald-state-{}-{unique}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("create the scratch directory");
    let state = DaemonState::open(&directory.join("registry.sqlite3")).expect("open");
    (Arc::new(state), directory)
}

/// A registry call that never returns leaves nothing recorded in the store, so
/// the daemon's exit status would report a clean stop unless the state handle
/// remembers it itself.
#[tokio::test]
async fn a_registry_call_that_cannot_complete_stops_the_daemon_vouching() {
    let (state, directory) = registry("panicking-call");
    assert!(!state.stopped_vouching());

    let outcome: Result<(), StateError> =
        state.off_the_reactor(|_| panic!("a store call died")).await;

    assert!(outcome.expect_err("fatal").is_fatal());
    assert!(
        state.stopped_vouching(),
        "the conclusion outlives the call that could not reach the store"
    );
    let _ = std::fs::remove_dir_all(&directory);
}

/// The ordinary path leaves it alone.
#[tokio::test]
async fn a_healthy_registry_keeps_vouching() {
    let (state, directory) = registry("healthy");

    assert_eq!(state.vouch().await.expect("vouched"), Vouched::Yes);

    assert!(!state.stopped_vouching());
    let _ = std::fs::remove_dir_all(&directory);
}
