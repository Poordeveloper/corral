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
    let (manifest, _) = crate::detection::manifest::parse(
        "schema = 1\nmin_engine_version = 1\nversion = \"t\"\nprovider = \"test\"\n\
         sealed_versions = [\"2.1.258\"]\n\
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
        .detect_with(Arc::new(manifest), Some("2.1.258".to_owned()));
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

/// What the tick writes down is what the derivation concluded: the claim the
/// state rests on, its horizon, how far past it a rot ran, and the provider
/// version bound to the runtime that produced the evidence. A record missing
/// them is a day of evidence that cannot answer why a state changed
/// (ADR 0015 D8, grill Q15).
#[test]
fn a_journaled_transition_carries_the_horizon_the_expiry_and_the_version() {
    let rotted = Change {
        session: CorralSessionId::mint(),
        from: MainState::Working,
        to: MainState::Unknown,
        transition: crate::attention::Transition::StateChanged {
            from: MainState::Working,
            to: MainState::Unknown,
        },
        decided_by: Some(corral_core::Claim {
            source: corral_core::EvidenceSource::PtyActivity,
            association: corral_core::Assurance::Deterministic,
            channel: corral_core::Channel::CorralOwnedPty,
            sealing: corral_core::Sealing::Sealed,
            asserts: corral_core::SemanticState::Working,
        }),
        horizon: Some(Duration::from_secs(3)),
        expired_after: Some(Duration::from_millis(400)),
        at: SystemTime::UNIX_EPOCH,
    };

    let crate::attention::Record::Transition(written) = record(&rotted, Some("2.1.258".to_owned()))
    else {
        panic!("a transition record");
    };
    assert_eq!(written.horizon, Some(Duration::from_secs(3)));
    assert_eq!(written.expired_after, Some(Duration::from_millis(400)));
    assert_eq!(written.provider_version.as_deref(), Some("2.1.258"));
    assert_eq!(
        written.source,
        Some(corral_core::EvidenceSource::PtyActivity)
    );
    assert_eq!(written.sealed, Some(true));
    assert_eq!(
        written.contradicted_first,
        Some(false),
        "the horizon ran out; nothing contradicted it"
    );

    let contradicted = Change {
        to: MainState::Ready,
        expired_after: None,
        ..rotted
    };
    let crate::attention::Record::Transition(written) = record(&contradicted, None) else {
        panic!("a transition record");
    };
    assert_eq!(written.contradicted_first, Some(true));
    assert_eq!(written.provider_version, None);
}

/// A move straight from one actionable state to another is two lifecycle
/// facts, and the record carries both: the item that ended, how it ended, and
/// the item that replaced it. Recording only the birth would leave the day's
/// evidence unable to say when the blocker was resolved (ADR 0015 D8).
#[test]
fn a_journaled_replacement_names_both_the_item_that_ended_and_the_one_born() {
    let ended = corral_core::AttentionItemId::mint();
    let born = corral_core::AttentionItemId::mint();
    let replaced = Change {
        session: CorralSessionId::mint(),
        from: MainState::NeedsYou,
        to: MainState::Ready,
        transition: crate::attention::Transition::ItemReplaced {
            ended,
            end: crate::attention::ItemEnd::Resolved,
            born,
        },
        decided_by: None,
        horizon: None,
        expired_after: None,
        at: SystemTime::UNIX_EPOCH,
    };
    let crate::attention::Record::Transition(written) = record(&replaced, None) else {
        panic!("a transition record");
    };
    assert_eq!(written.born, Some(born));
    assert_eq!(written.ended, Some(ended));
    assert_eq!(written.item_end, Some(crate::attention::ItemEnd::Resolved));
    assert!(written.notifiable, "the new item is one to ring for");
}
