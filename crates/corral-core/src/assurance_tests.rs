use super::*;

#[test]
fn heuristic_is_the_only_level_that_cannot_control() {
    assert!(Assurance::Deterministic.permits_control());
    assert!(Assurance::Attested.permits_control());
    assert!(Assurance::Manual.permits_control());
    assert!(!Assurance::Heuristic.permits_control());
}

#[test]
fn heuristic_is_the_only_level_that_cannot_assert_a_durable_fact() {
    assert!(Assurance::Deterministic.may_assert_durable_fact());
    assert!(Assurance::Attested.may_assert_durable_fact());
    assert!(Assurance::Manual.may_assert_durable_fact());
    assert!(!Assurance::Heuristic.may_assert_durable_fact());
}
