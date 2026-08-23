use std::time::SystemTime;

use super::*;

/// A hook having fired says a provider session exists, never that a runtime
/// is alive — the distinction ADR 0002 D2 draws to keep semantic evidence out
/// of runtime truth.
#[test]
fn semantic_evidence_never_establishes_a_runtime_occurrence() {
    for source in [
        EvidenceSource::ProviderHook,
        EvidenceSource::InBandSignal,
        EvidenceSource::ScreenDetection,
        EvidenceSource::HistoryRecord,
        EvidenceSource::Correlation,
        EvidenceSource::UserAssertion,
    ] {
        assert!(
            !source.establishes_runtime_occurrence(),
            "{source:?} must not mint a Run"
        );
    }
}

#[test]
fn construction_and_node_runtime_observation_establish_a_runtime_occurrence() {
    assert!(EvidenceSource::CorralConstructed.establishes_runtime_occurrence());
    assert!(EvidenceSource::NodeRuntimeObservation.establishes_runtime_occurrence());
}

/// A source that can prove a runtime exists still says nothing about which
/// Session it belongs to: the two axes are independent, which is what lets a
/// known runtime sit under a Heuristic binding.
#[test]
fn the_source_does_not_decide_the_assurance() {
    let observed = SystemTime::UNIX_EPOCH;
    let weak = Evidence::new(
        EvidenceSource::NodeRuntimeObservation,
        Assurance::Heuristic,
        observed,
    );

    assert!(weak.source().establishes_runtime_occurrence());
    assert!(!weak.assurance().permits_control());
}
