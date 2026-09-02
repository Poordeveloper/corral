use std::time::{Duration, SystemTime};

use corral_core::{
    Assurance, AttentionState, Channel, Claim, EvidenceSource, LastKnown, MainState, Sealing,
    SemanticState,
};

use super::*;
use crate::runtime::ExecutionState;

const NOW: SystemTime = SystemTime::UNIX_EPOCH;

fn at(seconds_ago: u64) -> SystemTime {
    NOW - Duration::from_secs(seconds_ago)
}

fn observed(
    source: EvidenceSource,
    asserts: SemanticState,
    ordinal: u64,
    seconds_ago: u64,
) -> Observed {
    Observed {
        claim: Claim {
            source,
            association: Assurance::Deterministic,
            channel: Channel::CorralOwnedPty,
            sealing: Sealing::Sealed,
            asserts,
        },
        observed_at: at(seconds_ago),
        ordinal,
    }
}

fn derive_running(claims: &[Observed]) -> Derived {
    derive(ExecutionState::Running, claims, &Horizons::default(), NOW)
}

/// Execution gates semantics (ADR 0015 D2): a runtime that ended has nothing
/// left to be Needs You about.
#[test]
fn exited_execution_is_exited_whatever_the_claims_say() {
    let blocked = observed(EvidenceSource::ProviderHook, SemanticState::NeedsYou, 1, 1);
    let derived = derive(
        ExecutionState::Exited,
        &[blocked],
        &Horizons::default(),
        NOW,
    );
    assert_eq!(derived.main, MainState::Exited);
    assert_eq!(derived.last_known, None);
}

/// A runtime Corral cannot place makes no semantic claim, but keeps the last
/// reliable fact for the secondary line.
#[test]
fn unknown_execution_is_unknown_with_the_newest_entitled_claim_as_last_known() {
    let blocked = observed(EvidenceSource::ProviderHook, SemanticState::NeedsYou, 1, 1);
    let derived = derive(
        ExecutionState::Unknown,
        &[blocked],
        &Horizons::default(),
        NOW,
    );
    assert_eq!(derived.main, MainState::Unknown);
    assert_eq!(
        derived.last_known,
        Some(LastKnown::new(MainState::NeedsYou, at(1)))
    );
}

#[test]
fn no_claims_is_unknown_with_nothing_last_known() {
    let derived = derive_running(&[]);
    assert_eq!(derived.main, MainState::Unknown);
    assert_eq!(derived.last_known, None);
}

/// Among fresh entitled claims the causally newest wins, however much more
/// authoritative the older one was (grill Q3).
#[test]
fn the_causally_newest_fresh_claim_wins_over_authority() {
    let older_hook = observed(EvidenceSource::ProviderHook, SemanticState::NeedsYou, 1, 10);
    let newer_screen = observed(EvidenceSource::ScreenDetection, SemanticState::Ready, 2, 1);
    assert_eq!(
        derive_running(&[older_hook, newer_screen]).main,
        MainState::Ready
    );
}

/// Ordering is Corral's observation sequence, never a comparison of wall
/// clocks: a clock that stepped back must not make the later fact older.
#[test]
fn ordering_is_by_observation_sequence_not_wall_clock() {
    let later_by_sequence = observed(EvidenceSource::ProviderHook, SemanticState::Ready, 2, 5);
    let earlier_by_sequence = observed(EvidenceSource::ProviderHook, SemanticState::Working, 1, 1);
    assert_eq!(
        derive_running(&[earlier_by_sequence, later_by_sequence]).main,
        MainState::Ready
    );
}

/// Activity is the default and a blocker is the exception: the prompt that
/// blocks the agent is drawn by the same output flow (ADR 0015 D4).
#[test]
fn a_fresh_blocker_beats_fresh_activity() {
    let blocker = observed(
        EvidenceSource::ScreenDetection,
        SemanticState::NeedsYou,
        2,
        1,
    );
    let activity = observed(EvidenceSource::PtyActivity, SemanticState::Working, 3, 0);
    assert_eq!(
        derive_running(&[blocker, activity]).main,
        MainState::NeedsYou
    );
}

#[test]
fn activity_alone_is_working_until_the_quiet_horizon() {
    let recent = observed(EvidenceSource::PtyActivity, SemanticState::Working, 1, 2);
    assert_eq!(derive_running(&[recent]).main, MainState::Working);

    let quiet = observed(EvidenceSource::PtyActivity, SemanticState::Working, 1, 4);
    let derived = derive_running(&[quiet]);
    assert_eq!(derived.main, MainState::Unknown);
    assert_eq!(
        derived.last_known,
        Some(LastKnown::new(MainState::Working, at(4)))
    );
}

/// Every semantic claim rots (ADR 0015 D4): past its horizon the main state
/// is Unknown and the claim survives only as the last known fact.
#[test]
fn a_claim_past_its_horizon_rots_to_unknown_with_last_known() {
    let stale = observed(
        EvidenceSource::ProviderHook,
        SemanticState::NeedsYou,
        1,
        6 * 60,
    );
    let derived = derive_running(&[stale]);
    assert_eq!(derived.main, MainState::Unknown);
    assert_eq!(
        derived.last_known,
        Some(LastKnown::new(MainState::NeedsYou, at(6 * 60)))
    );

    let fresh = observed(
        EvidenceSource::ProviderHook,
        SemanticState::NeedsYou,
        1,
        4 * 60,
    );
    assert_eq!(derive_running(&[fresh]).main, MainState::NeedsYou);
}

/// A claim nobody is entitled to make is not a fact that rotted; it never
/// was one, so it is not even last known.
#[test]
fn unentitled_claims_are_ignored_entirely() {
    let mut heuristic = observed(EvidenceSource::ProviderHook, SemanticState::NeedsYou, 1, 1);
    heuristic.claim.association = Assurance::Heuristic;
    let mut unsealed = observed(
        EvidenceSource::ScreenDetection,
        SemanticState::NeedsYou,
        2,
        1,
    );
    unsealed.claim.sealing = Sealing::Unsealed;
    let derived = derive_running(&[heuristic, unsealed]);
    assert_eq!(derived.main, MainState::Unknown);
    assert_eq!(derived.last_known, None);
}

/// The ruled initial horizons (grill Q15), stated so a change to one is a
/// change someone made on purpose.
#[test]
fn the_default_horizons_are_the_ruled_initial_values() {
    let horizons = Horizons::default();
    assert_eq!(
        horizons.of(EvidenceSource::PtyActivity, SemanticState::Working),
        Duration::from_secs(3)
    );
    assert_eq!(
        horizons.of(EvidenceSource::ProviderHook, SemanticState::Working),
        Duration::from_secs(15 * 60)
    );
    assert_eq!(
        horizons.of(EvidenceSource::ProviderHook, SemanticState::NeedsYou),
        Duration::from_secs(5 * 60)
    );
    assert_eq!(
        horizons.of(EvidenceSource::ProviderHook, SemanticState::Ready),
        Duration::from_secs(2 * 60 * 60)
    );
}

/// The derived shape converts into the state a client reads.
#[test]
fn a_derivation_becomes_an_attention_state_at_an_instant() {
    let derived = derive_running(&[observed(
        EvidenceSource::ProviderHook,
        SemanticState::Ready,
        1,
        1,
    )]);
    assert_eq!(
        derived.into_state(NOW),
        AttentionState::asserted(MainState::Ready, NOW)
    );
    let rotted = derive_running(&[observed(
        EvidenceSource::ProviderHook,
        SemanticState::Ready,
        1,
        3 * 60 * 60,
    )]);
    assert_eq!(
        rotted.into_state(NOW),
        AttentionState::unknown(NOW, Some(LastKnown::new(MainState::Ready, at(3 * 60 * 60))))
    );
}
