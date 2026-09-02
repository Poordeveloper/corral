use std::time::{Duration, SystemTime};

use corral_core::{
    Assurance, AttentionItemId, Channel, Claim, CorralSessionId, EvidenceSource, MainState,
    Sealing, SemanticState,
};

use super::*;
use crate::attention::{Acknowledgement, ItemEnd, Transition};
use crate::runtime::ExecutionState;

const T0: SystemTime = SystemTime::UNIX_EPOCH;

fn later(seconds: u64) -> SystemTime {
    T0 + Duration::from_secs(seconds)
}

fn hook(asserts: SemanticState) -> Claim {
    Claim {
        source: EvidenceSource::ProviderHook,
        association: Assurance::Deterministic,
        channel: Channel::CorralOwnedPty,
        sealing: Sealing::Sealed,
        asserts,
    }
}

fn activity() -> Claim {
    Claim {
        source: EvidenceSource::PtyActivity,
        association: Assurance::Deterministic,
        channel: Channel::CorralOwnedPty,
        sealing: Sealing::Sealed,
        asserts: SemanticState::Working,
    }
}

fn running(_: CorralSessionId) -> ExecutionState {
    ExecutionState::Running
}

/// A claim observed and a tick later: the state is derived, the item born,
/// and the tick reports the transition for the journal.
#[test]
fn a_tick_derives_the_state_and_reports_the_transition() {
    let mut ledger = Ledger::new(Horizons::default());
    let session = CorralSessionId::mint();
    ledger.observe(session, hook(SemanticState::NeedsYou), later(1));
    let changes = ledger.tick(later(2), running);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].session, session);
    assert_eq!(changes[0].from, MainState::Unknown);
    assert_eq!(changes[0].to, MainState::NeedsYou);
    assert!(matches!(changes[0].transition, Transition::ItemBorn(_)));
    let (state, item) = ledger.state(session).expect("tracked");
    assert_eq!(state.main(), MainState::NeedsYou);
    assert_eq!(state.since(), later(2));
    assert!(item.is_some());

    assert!(ledger.tick(later(3), running).is_empty(), "nothing changed");
}

/// Observation order is the daemon's sequence: the later observation wins
/// even when its clock reads earlier.
#[test]
fn the_ledger_keeps_the_newest_claim_per_source_and_state() {
    let mut ledger = Ledger::new(Horizons::default());
    let session = CorralSessionId::mint();
    ledger.observe(session, hook(SemanticState::NeedsYou), later(5));
    ledger.observe(session, hook(SemanticState::Ready), later(4));
    ledger.tick(later(6), running);
    assert_eq!(
        ledger.state(session).expect("tracked").0.main(),
        MainState::Ready
    );
}

/// Re-observing a claim is a new observation: a screen that still shows the
/// blocker keeps it the newest claim (ADR 0015 D4).
#[test]
fn re_observation_makes_a_claim_newest_again() {
    let mut ledger = Ledger::new(Horizons::default());
    let session = CorralSessionId::mint();
    let screen = Claim {
        source: EvidenceSource::ScreenDetection,
        ..hook(SemanticState::NeedsYou)
    };
    ledger.observe(session, screen, later(1));
    ledger.observe(session, hook(SemanticState::Working), later(2));
    ledger.observe(session, screen, later(3));
    ledger.tick(later(4), running);
    assert_eq!(
        ledger.state(session).expect("tracked").0.main(),
        MainState::NeedsYou
    );
}

#[test]
fn activity_yields_to_a_blocker_and_rots_at_the_quiet_horizon() {
    let mut ledger = Ledger::new(Horizons::default());
    let session = CorralSessionId::mint();
    ledger.observe(session, activity(), later(10));
    ledger.tick(later(11), running);
    assert_eq!(
        ledger.state(session).expect("tracked").0.main(),
        MainState::Working
    );
    let changes = ledger.tick(later(14), running);
    assert_eq!(changes[0].to, MainState::Unknown);
    assert_eq!(
        changes[0].transition,
        Transition::StateChanged {
            from: MainState::Working,
            to: MainState::Unknown
        }
    );
}

#[test]
fn the_summary_counts_totals_and_unacknowledged_per_class() {
    let mut ledger = Ledger::new(Horizons::default());
    let blocked = [
        CorralSessionId::mint(),
        CorralSessionId::mint(),
        CorralSessionId::mint(),
    ];
    let ready = CorralSessionId::mint();
    for session in blocked {
        ledger.observe(session, hook(SemanticState::NeedsYou), later(1));
    }
    ledger.observe(ready, hook(SemanticState::Ready), later(1));
    ledger.tick(later(2), running);
    let item = ledger
        .state(blocked[0])
        .expect("tracked")
        .1
        .expect("item")
        .id();
    assert_eq!(
        ledger.acknowledge(blocked[0], item),
        Acknowledgement::Acknowledged
    );
    let summary = ledger.summary();
    assert_eq!(
        (summary.needs_you.total, summary.needs_you.unacknowledged),
        (3, 2)
    );
    assert_eq!((summary.ready.total, summary.ready.unacknowledged), (1, 1));
}

#[test]
fn acknowledging_an_unknown_session_or_a_stale_item_changes_nothing() {
    let mut ledger = Ledger::new(Horizons::default());
    let session = CorralSessionId::mint();
    assert_eq!(
        ledger.acknowledge(session, AttentionItemId::mint()),
        Acknowledgement::NoCurrentItem
    );
    ledger.observe(session, hook(SemanticState::NeedsYou), later(1));
    ledger.tick(later(2), running);
    assert_eq!(
        ledger.acknowledge(session, AttentionItemId::mint()),
        Acknowledgement::StaleAttentionItem
    );
    assert_eq!(ledger.summary().needs_you.unacknowledged, 1);
}

/// A successful open acknowledges Ready and not Needs You, through the
/// ledger, so the terminal channel needs no knowledge of items.
#[test]
fn opening_acknowledges_a_ready_session_only() {
    let mut ledger = Ledger::new(Horizons::default());
    let ready = CorralSessionId::mint();
    let blocked = CorralSessionId::mint();
    ledger.observe(ready, hook(SemanticState::Ready), later(1));
    ledger.observe(blocked, hook(SemanticState::NeedsYou), later(1));
    ledger.tick(later(2), running);
    ledger.opened(ready);
    ledger.opened(blocked);
    let summary = ledger.summary();
    assert_eq!(summary.ready.unacknowledged, 0);
    assert_eq!(summary.needs_you.unacknowledged, 1);
}

/// An ended runtime ends the item with the reason a person is told.
#[test]
fn exit_ends_the_item() {
    let mut ledger = Ledger::new(Horizons::default());
    let session = CorralSessionId::mint();
    ledger.observe(session, hook(SemanticState::NeedsYou), later(1));
    ledger.tick(later(2), running);
    let changes = ledger.tick(later(3), |_| ExecutionState::Exited);
    assert!(matches!(
        changes[0].transition,
        Transition::ItemEnded {
            end: ItemEnd::Exited,
            ..
        }
    ));
    assert_eq!(
        ledger.state(session).expect("tracked").0.main(),
        MainState::Exited
    );
}

/// The change record carries what the journal needs: the claim's source,
/// association, and sealing when a claim decided the state.
#[test]
fn a_change_names_the_claim_that_decided_it() {
    let mut ledger = Ledger::new(Horizons::default());
    let session = CorralSessionId::mint();
    ledger.observe(session, hook(SemanticState::Ready), later(1));
    let changes = ledger.tick(later(2), running);
    assert_eq!(
        changes[0].decided_by.map(|claim| claim.source),
        Some(EvidenceSource::ProviderHook)
    );
    assert_eq!(
        changes[0].decided_by.map(|claim| claim.sealing),
        Some(Sealing::Sealed)
    );
}
