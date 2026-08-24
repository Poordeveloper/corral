use corral_core::ExitCause;

use super::*;

fn exited(run: RunId) -> RunOccurrence {
    RunOccurrence::Exited {
        run,
        end: RunEnd::Exited(ExitCause::Completed),
        at: OccurrenceTime::Unknown,
    }
}

#[test]
fn what_is_reported_is_what_is_drained() {
    let (observations, observed) = observe_runs();
    let run = RunId::mint();

    observations.report(exited(run));

    let Some(first) = observed.next() else {
        panic!("a reported occurrence is drained");
    };
    assert_eq!(first.occurrence(), exited(run));
    assert_eq!(observations.integrity(), Integrity::Intact);
}

/// Queue exhaustion is an integrity failure, not backpressure. A lifecycle
/// fact that cannot be recorded leaves a durable Run open forever, and the
/// daemon's answer is to stop rather than to carry on (grill Q10).
#[test]
fn a_queue_that_cannot_take_a_fact_loses_integrity_rather_than_the_fact_quietly() {
    let (observations, observed) = observe_runs();
    // Nothing drains, so the queue fills at exactly its stated capacity.
    for _ in 0..OBSERVATION_QUEUE {
        observations.report(exited(RunId::mint()));
    }
    assert_eq!(
        observations.integrity(),
        Integrity::Intact,
        "a full queue is not yet a lost one"
    );

    observations.report(exited(RunId::mint()));

    assert_eq!(observations.integrity(), Integrity::Lost);
    // Held so the channel is not closed by the receiver going away, which
    // would be a different way to lose a fact than the one under test.
    drop(observed);
}

/// Reporting never waits, whatever is or is not draining. The reaper and the
/// retiring screen call this while tearing a session down.
#[test]
fn reporting_never_blocks_when_nothing_is_draining() {
    let (observations, observed) = observe_runs();
    let reporting = std::thread::spawn(move || {
        for _ in 0..OBSERVATION_QUEUE * 2 {
            observations.report(exited(RunId::mint()));
        }
    });

    reporting
        .join()
        .expect("reporting completes without a drainer");
    drop(observed);
}

/// A shutdown waits for what it has already reported, and a wait that runs out
/// is itself the answer: facts left in the queue at exit are facts nobody will
/// ever write.
#[test]
fn settling_reports_whether_everything_reported_was_recorded() {
    let (observations, observed) = observe_runs();
    observations.report(exited(RunId::mint()));

    assert_eq!(
        observations.settle(std::time::Duration::from_millis(50)),
        Integrity::Lost,
        "nothing drained it, so the fact was never recorded"
    );

    let (observations, observed_again) = observe_runs();
    observations.report(exited(RunId::mint()));
    drop(observed_again.next());
    assert_eq!(
        observations.settle(std::time::Duration::from_millis(50)),
        Integrity::Intact
    );
    drop(observed);
}
