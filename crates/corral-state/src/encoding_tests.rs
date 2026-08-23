use super::*;

#[test]
fn every_assurance_round_trips_through_its_durable_token() {
    for assurance in [
        Assurance::Deterministic,
        Assurance::Attested,
        Assurance::Manual,
        Assurance::Heuristic,
    ] {
        let token = assurance_token(assurance);
        assert_eq!(assurance_from_token(token), Ok(assurance));
    }
}

#[test]
fn every_evidence_source_round_trips_through_its_durable_token() {
    for source in [
        EvidenceSource::CorralConstructed,
        EvidenceSource::NodeRuntimeObservation,
        EvidenceSource::ProviderHook,
        EvidenceSource::InBandSignal,
        EvidenceSource::ScreenDetection,
        EvidenceSource::HistoryRecord,
        EvidenceSource::Correlation,
        EvidenceSource::UserAssertion,
    ] {
        assert_eq!(
            evidence_source_from_token(evidence_source_token(source)),
            Ok(source)
        );
    }
}

#[test]
fn every_run_end_round_trips_through_its_durable_token() {
    for end in [
        RunEnd::Exited(ExitCause::Completed),
        RunEnd::Exited(ExitCause::Failed),
        RunEnd::Exited(ExitCause::Terminated),
        RunEnd::Exited(ExitCause::Unknown),
        RunEnd::Unverifiable,
    ] {
        assert_eq!(run_end_from_token(run_end_token(end)), Ok(end));
    }
}

/// An unverifiable end and an exit whose cause was not determined are
/// different facts, and the durable tokens keep them different.
#[test]
fn an_unverifiable_end_is_not_stored_as_an_exit() {
    assert_ne!(
        run_end_token(RunEnd::Unverifiable),
        run_end_token(RunEnd::Exited(ExitCause::Unknown))
    );
}

/// A token this build does not know is a fact it cannot read. Guessing would
/// make the projection silently incomplete.
#[test]
fn an_unknown_token_is_unreadable_rather_than_guessed() {
    let error = assurance_from_token("probable").expect_err("unreadable");

    assert!(matches!(error, FatalState::Unreadable { .. }));
}

#[test]
fn instants_round_trip_on_both_sides_of_the_epoch() {
    for at in [
        SystemTime::UNIX_EPOCH,
        SystemTime::UNIX_EPOCH + Duration::from_millis(1_766_000_000_123),
        SystemTime::UNIX_EPOCH - Duration::from_millis(86_400_000),
    ] {
        let stored = millis(at).expect("representable");
        assert_eq!(from_millis(stored), at);
    }
}

/// A real clock carries nanoseconds. Rounding at the boundary is what keeps
/// a value the store returns equal to the value it later reads back.
#[test]
fn an_instant_finer_than_the_store_rounds_to_what_will_be_read_back() {
    let precise = SystemTime::UNIX_EPOCH + Duration::new(1_766_000_000, 123_456_789);

    let stored = as_stored(precise).expect("representable");

    assert_ne!(stored, precise);
    assert_eq!(stored, from_millis(millis(precise).expect("representable")));
    assert_eq!(as_stored(stored).expect("representable"), stored);
}

#[test]
fn a_clock_beyond_the_stored_range_is_refused() {
    let absurd = SystemTime::UNIX_EPOCH + Duration::from_secs(u64::MAX / 1_000);

    assert_eq!(millis(absurd), Err(FatalState::UnrepresentableTime));
}
