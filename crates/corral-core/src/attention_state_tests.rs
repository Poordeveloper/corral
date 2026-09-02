use std::time::{Duration, SystemTime};

use super::*;

/// A last reliable fact is shown beneath Unknown and nowhere else
/// (`PRODUCT.md` §4): the constructors make any other pairing unwritable.
#[test]
fn a_last_known_fact_exists_only_beneath_unknown() {
    let then = SystemTime::UNIX_EPOCH;
    let rotted = AttentionState::unknown(
        then + Duration::from_secs(60),
        Some(LastKnown::new(MainState::NeedsYou, then)),
    );
    assert_eq!(rotted.main(), MainState::Unknown);
    assert_eq!(
        rotted.last_known().map(|known| known.state()),
        Some(MainState::NeedsYou)
    );

    let working = AttentionState::asserted(MainState::Working, then);
    assert_eq!(working.last_known(), None);
}

/// Exited and Unknown are not semantic assertions; `asserted` refuses them so
/// a caller cannot manufacture "Exited since" from a semantic claim.
#[test]
fn only_the_three_semantic_states_can_be_asserted() {
    assert!(MainState::Working.is_semantic());
    assert!(MainState::NeedsYou.is_semantic());
    assert!(MainState::Ready.is_semantic());
    assert!(!MainState::Unknown.is_semantic());
    assert!(!MainState::Exited.is_semantic());
}

/// Every semantic state maps to the claim a source makes, and back.
#[test]
fn semantic_states_round_trip_through_main_states() {
    for semantic in [
        SemanticState::Working,
        SemanticState::NeedsYou,
        SemanticState::Ready,
    ] {
        let main = MainState::from(semantic);
        assert_eq!(SemanticState::try_from(main), Ok(semantic));
    }
    assert!(SemanticState::try_from(MainState::Unknown).is_err());
    assert!(SemanticState::try_from(MainState::Exited).is_err());
}
