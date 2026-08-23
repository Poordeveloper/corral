use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use super::*;

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A registry on a real file, cleaned up however the test ends — one of these
/// tests is about a panic, and a scratch directory left behind by a failing
/// run is the one nobody goes back for.
struct Registry {
    state: Arc<DaemonState>,
    directory: PathBuf,
}

impl Drop for Registry {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn registry(name: &str) -> Registry {
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "corrald-state-{}-{unique}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("create the scratch directory");
    let state = DaemonState::open(&directory.join("registry.sqlite3")).expect("open");
    Registry {
        state: Arc::new(state),
        directory,
    }
}

/// A registry call that never returns leaves nothing recorded in the store, so
/// the daemon's exit status would report a clean stop unless the state handle
/// remembers it itself.
#[tokio::test]
async fn a_registry_call_that_cannot_complete_stops_the_daemon_vouching() {
    let registry = registry("panicking-call");
    assert!(!registry.state.stopped_vouching());

    let outcome: Result<(), StateError> = registry
        .state
        .off_the_reactor(|_| panic!("a store call died"))
        .await;

    assert!(outcome.expect_err("fatal").is_fatal());
    assert!(
        registry.state.stopped_vouching(),
        "the conclusion outlives the call that could not reach the store"
    );
}

/// The ordinary path leaves it alone.
#[tokio::test]
async fn a_healthy_registry_keeps_vouching() {
    let registry = registry("healthy");

    assert_eq!(registry.state.vouch().await.expect("vouched"), Vouched::Yes);

    assert!(!registry.state.stopped_vouching());
}
