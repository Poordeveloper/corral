use std::time::Duration;

use super::*;

fn run(started: OccurrenceTime) -> Run {
    Run::started(
        RunId::mint(),
        CorralSessionId::mint(),
        BindingId::mint(),
        started,
    )
}

/// The whole reason the occurrence time is an enum: a first-observed instant
/// can be carried, but it can never be read as a start time.
#[test]
fn a_first_observed_time_is_never_an_authoritative_start() {
    let observed = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
    let weak = run(OccurrenceTime::FirstObserved(observed));

    assert_eq!(weak.started_at(), OccurrenceTime::FirstObserved(observed));
    assert_eq!(weak.started_at().authoritative(), None);
}

#[test]
fn an_unknown_start_offers_no_instant_at_all() {
    let unknown = run(OccurrenceTime::Unknown);

    assert_eq!(unknown.started_at(), OccurrenceTime::Unknown);
    assert_eq!(unknown.started_at().authoritative(), None);
}

#[test]
fn an_authoritative_start_is_readable_as_one() {
    let at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);

    assert_eq!(
        run(OccurrenceTime::Authoritative(at))
            .started_at()
            .authoritative(),
        Some(at)
    );
}

#[test]
fn a_run_is_live_until_it_ends() {
    let live = run(OccurrenceTime::Authoritative(SystemTime::UNIX_EPOCH));
    assert!(live.is_live());
    assert_eq!(live.end(), None);

    let ended = live.ended(
        RunEnd::Exited(ExitCause::Completed),
        OccurrenceTime::Authoritative(SystemTime::UNIX_EPOCH),
    );
    assert!(!ended.is_live());
    assert_eq!(ended.end(), Some(RunEnd::Exited(ExitCause::Completed)));
}

/// Unverifiable is a first-class end, distinct from every observed exit —
/// unreachable is never reported as exited.
#[test]
fn an_unverifiable_end_is_not_an_exit() {
    let ended = run(OccurrenceTime::Authoritative(SystemTime::UNIX_EPOCH)).ended(
        RunEnd::Unverifiable,
        OccurrenceTime::FirstObserved(SystemTime::UNIX_EPOCH),
    );

    assert_eq!(ended.end(), Some(RunEnd::Unverifiable));
    assert!(!matches!(ended.end(), Some(RunEnd::Exited(_))));
}

/// A Run nobody is numbering has no number. An invented one would name a
/// position another Run already occupies.
#[test]
fn a_run_arrives_without_a_position() {
    let unnumbered = run(OccurrenceTime::Unknown);

    assert_eq!(unnumbered.ordinal(), None);
    assert_eq!(
        unnumbered.with_ordinal(RunOrdinal::FIRST).ordinal(),
        Some(RunOrdinal::FIRST)
    );
}

#[test]
fn ordinals_count_from_one_and_do_not_wrap() {
    assert_eq!(RunOrdinal::FIRST.position(), 1);
    assert_eq!(RunOrdinal::FIRST.next().position(), 2);
    assert_eq!(
        RunOrdinal::from_position(u32::MAX).next().position(),
        u32::MAX
    );
}
