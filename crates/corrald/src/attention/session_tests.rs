use std::time::{Duration, SystemTime};

use corral_core::{AttentionReason, LastKnown, MainState};

use super::*;
use crate::attention::Derived;

const T0: SystemTime = SystemTime::UNIX_EPOCH;

fn later(seconds: u64) -> SystemTime {
    T0 + Duration::from_secs(seconds)
}

fn asserted(main: MainState) -> Derived {
    Derived {
        main,
        last_known: None,
        rests_on: None,
    }
}

fn unknown(last_known: Option<LastKnown>) -> Derived {
    Derived {
        main: MainState::Unknown,
        last_known,
        rests_on: None,
    }
}

/// A session starts Unknown: nothing has been derived yet, and that is the
/// honest state, not a placeholder.
#[test]
fn a_new_session_reads_unknown_since_its_creation() {
    let session = SessionAttention::new(T0);
    assert_eq!(session.state().main(), MainState::Unknown);
    assert_eq!(session.state().since(), T0);
    assert_eq!(session.item(), None);
}

/// `since` is when the main state was entered, not when it was last confirmed.
#[test]
fn since_moves_only_when_the_main_state_changes() {
    let mut session = SessionAttention::new(T0);
    session.apply(asserted(MainState::Working), later(1));
    session.apply(asserted(MainState::Working), later(5));
    assert_eq!(session.state().since(), later(1));
    session.apply(asserted(MainState::Ready), later(6));
    assert_eq!(session.state().since(), later(6));
}

/// Entering Needs You or Ready births an item with a fresh identity; the
/// transition says so, once.
#[test]
fn entering_needs_you_births_an_item_once() {
    let mut session = SessionAttention::new(T0);
    let born = session.apply(asserted(MainState::NeedsYou), later(1));
    let item = session.item().expect("an item");
    assert_eq!(item.reason(), AttentionReason::NeedsInput);
    assert!(!item.acknowledged());
    assert_eq!(born, Transition::ItemBorn(item.id()));

    let again = session.apply(asserted(MainState::NeedsYou), later(2));
    assert_eq!(again, Transition::Unchanged);
    assert_eq!(session.item().map(|item| item.id()), Some(item.id()));
}

#[test]
fn entering_ready_births_a_turn_complete_item() {
    let mut session = SessionAttention::new(T0);
    session.apply(asserted(MainState::Ready), later(1));
    assert_eq!(
        session.item().map(|item| item.reason()),
        Some(AttentionReason::TurnComplete)
    );
}

/// Leaving the state ends the item; re-entering mints a new one and re-arms
/// (grill Q19).
#[test]
fn leaving_and_re_entering_mints_a_new_item() {
    let mut session = SessionAttention::new(T0);
    session.apply(asserted(MainState::NeedsYou), later(1));
    let first = session.item().expect("an item").id();
    let ended = session.apply(asserted(MainState::Working), later(2));
    assert_eq!(
        ended,
        Transition::ItemEnded {
            item: first,
            end: ItemEnd::Resolved
        }
    );
    assert_eq!(session.item(), None);

    let reborn = session.apply(asserted(MainState::NeedsYou), later(3));
    let second = session.item().expect("an item").id();
    assert_ne!(first, second);
    assert_eq!(reborn, Transition::ItemBorn(second));
}

/// Rot invalidates the item without resolving it, and exit invalidates it
/// with the reason a person is told ("Exited before you responded").
#[test]
fn rot_and_exit_invalidate_rather_than_resolve() {
    let mut session = SessionAttention::new(T0);
    session.apply(asserted(MainState::NeedsYou), later(1));
    let item = session.item().expect("an item").id();
    let rotted = session.apply(
        unknown(Some(LastKnown::new(MainState::NeedsYou, later(1)))),
        later(400),
    );
    assert_eq!(
        rotted,
        Transition::ItemEnded {
            item,
            end: ItemEnd::Rotted
        }
    );
    assert_eq!(
        session.state().last_known(),
        Some(LastKnown::new(MainState::NeedsYou, later(1)))
    );

    let mut session = SessionAttention::new(T0);
    session.apply(asserted(MainState::NeedsYou), later(1));
    let item = session.item().expect("an item").id();
    let exited = session.apply(asserted(MainState::Exited), later(2));
    assert_eq!(
        exited,
        Transition::ItemEnded {
            item,
            end: ItemEnd::Exited
        }
    );
}

/// Acknowledgement names the item it saw; a stale id never acknowledges the
/// replacement (grill Q18).
#[test]
fn acknowledgement_is_by_item_and_a_stale_id_is_a_no_op() {
    let mut session = SessionAttention::new(T0);
    session.apply(asserted(MainState::NeedsYou), later(1));
    let first = session.item().expect("an item").id();
    session.apply(asserted(MainState::Working), later(2));
    session.apply(asserted(MainState::NeedsYou), later(3));
    let second = session.item().expect("an item").id();

    assert_eq!(
        session.acknowledge(first),
        Acknowledgement::StaleAttentionItem
    );
    assert!(!session.item().expect("an item").acknowledged());
    assert_eq!(session.acknowledge(second), Acknowledgement::Acknowledged);
    assert!(session.item().expect("an item").acknowledged());
    assert_eq!(session.acknowledge(second), Acknowledgement::Acknowledged);
}

#[test]
fn acknowledging_with_no_current_item_says_so() {
    let mut session = SessionAttention::new(T0);
    assert_eq!(
        session.acknowledge(corral_core::AttentionItemId::mint()),
        Acknowledgement::NoCurrentItem
    );
}

/// An acknowledged item keeps the row's state: only the badge changes.
#[test]
fn acknowledgement_does_not_change_the_main_state() {
    let mut session = SessionAttention::new(T0);
    session.apply(asserted(MainState::Ready), later(1));
    let item = session.item().expect("an item").id();
    session.acknowledge(item);
    assert_eq!(session.state().main(), MainState::Ready);
}

/// A successful Open acknowledges a Ready item and never a Needs You item
/// (grill Q18).
#[test]
fn a_successful_open_acknowledges_ready_but_not_needs_you() {
    let mut ready = SessionAttention::new(T0);
    ready.apply(asserted(MainState::Ready), later(1));
    ready.opened();
    assert!(ready.item().expect("an item").acknowledged());

    let mut blocked = SessionAttention::new(T0);
    blocked.apply(asserted(MainState::NeedsYou), later(1));
    blocked.opened();
    assert!(!blocked.item().expect("an item").acknowledged());
}

/// Needs You straight into Ready ends one item and births another in the same
/// derivation. Both are lifecycle facts, and neither is inferable from the
/// other: a transition that could carry only the birth drops every resolution
/// that happened this way, so the journal never records it (ADR 0015 D8).
#[test]
fn moving_between_actionable_states_reports_the_end_and_the_birth() {
    let mut session = SessionAttention::new(T0);
    session.apply(asserted(MainState::NeedsYou), later(1));
    let first = session.item().expect("an item").id();
    let transition = session.apply(asserted(MainState::Ready), later(2));
    let second = session.item().expect("an item").id();
    assert_ne!(first, second);
    assert_eq!(
        transition,
        Transition::ItemReplaced {
            ended: first,
            end: ItemEnd::Resolved,
            born: second,
        }
    );
}
