use std::sync::Arc;
use std::time::{Duration, SystemTime};

use corral_core::{CorralSessionId, MainState, RunId};

use super::*;
use crate::runtime::{LaunchRequest, PtyGeometry, spawn_session};
use crate::state::DaemonState;

fn daemon(name: &str) -> (Arc<DaemonState>, std::path::PathBuf) {
    let directory =
        std::env::temp_dir().join(format!("corrald-tick-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("scratch");
    let state = DaemonState::open(
        &directory.join("registry.sqlite3"),
        &directory.join("launch"),
        &directory,
    )
    .expect("open");
    (Arc::new(state), directory)
}

/// The chain from a screen to a row, with a sealed rule and nothing else:
/// the child draws, the screen thread reads it, the tick observes the
/// reading, derivation asserts, the ledger holds the state the list will
/// carry. Synthetic evidence proves the mechanics (grill Q32); the sealing
/// of real rules waits for the reconciliation.
#[test]
fn a_sealed_screen_reading_becomes_the_sessions_main_state() {
    let (state, directory) = daemon("sealed-reading");
    let (manifest, _) = crate::detection::parse(
        "schema = 1\nmin_engine_version = 1\nversion = \"t\"\nprovider = \"test\"\n\
         [[rule]]\nid = \"ready\"\nasserts = \"turn_complete\"\nregion = \"whole_screen\"\n\
         all = [\"done\"]\nsealed_by = \"synthetic\"\n",
    )
    .expect("manifest");
    let launch = LaunchRequest::new(
        "/bin/sh",
        ["-c", "printf done; sleep 30"]
            .iter()
            .map(std::ffi::OsString::from),
        std::env::temp_dir(),
    )
    .expect("launch");
    let pending = spawn_session(&launch, PtyGeometry::expect_valid(24, 80))
        .expect("spawn")
        .detect_with(Arc::new(manifest));
    let session = CorralSessionId::mint();
    let handle = pending.serve(session, RunId::mint(), state.observations().clone());
    state.with_runtime(|runtime| runtime.sessions.insert(handle));

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut main = MainState::Unknown;
    while main != MainState::Ready && std::time::Instant::now() < deadline {
        tick_once(&state, SystemTime::now());
        main = state
            .with_runtime(|runtime| {
                runtime
                    .attention
                    .state(session)
                    .map(|(state, _)| state.main())
            })
            .flatten()
            .unwrap_or(MainState::Unknown);
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(main, MainState::Ready);
    let item = state
        .with_runtime(|runtime| runtime.attention.state(session).and_then(|(_, item)| item))
        .flatten()
        .expect("a Ready item");
    assert_eq!(item.reason(), corral_core::AttentionReason::TurnComplete);

    state.with_runtime(|runtime| {
        if let Some(handle) = runtime.sessions.get(session) {
            handle.shut_down();
        }
    });
    let _ = std::fs::remove_dir_all(directory);
}
