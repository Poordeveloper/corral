use super::*;

/// S2 verified that an externally observed Claude fork reports only
/// `source: "fork"` and holds no reference to its parent, so message-uuid
/// overlap is all a discoverer has. This is the rule that forbids recording
/// it.
#[test]
fn heuristic_similarity_records_no_lineage() {
    let refusal = SessionLineage::record(
        CorralSessionId::mint(),
        CorralSessionId::mint(),
        Assurance::Heuristic,
    )
    .expect_err("refused");

    assert_eq!(
        refusal,
        LineageRefused::UnsupportedAssurance(Assurance::Heuristic)
    );
}

#[test]
fn a_corral_initiated_fork_records_the_edge() {
    let parent = CorralSessionId::mint();
    let child = CorralSessionId::mint();

    let edge = SessionLineage::record(child, parent, Assurance::Deterministic).expect("recorded");

    assert_eq!(edge.child(), child);
    assert_eq!(edge.parent(), parent);
    assert_eq!(edge.assurance(), Assurance::Deterministic);
}

#[test]
fn every_assurance_that_may_assert_a_durable_fact_may_record_lineage() {
    for assurance in [
        Assurance::Deterministic,
        Assurance::Attested,
        Assurance::Manual,
    ] {
        assert!(
            SessionLineage::record(CorralSessionId::mint(), CorralSessionId::mint(), assurance)
                .is_ok()
        );
    }
}

#[test]
fn a_session_cannot_continue_itself() {
    let session = CorralSessionId::mint();

    let refusal =
        SessionLineage::record(session, session, Assurance::Deterministic).expect_err("refused");

    assert_eq!(refusal, LineageRefused::SelfParent);
}
