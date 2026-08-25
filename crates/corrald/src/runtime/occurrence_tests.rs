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

/// Queue exhaustion is an integrity failure, not backpressure, for a fact the
/// daemon must account for: an unrecorded ending leaves a durable Run open
/// forever, and the daemon's answer is to stop rather than to carry on
/// (grill Q10).
#[test]
fn a_queue_that_cannot_take_an_ending_loses_integrity_rather_than_the_fact_quietly() {
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

/// Attachment activity is advisory. A client connecting and disconnecting in a
/// loop is an observer's behaviour, and it may not reach the daemon's
/// lifecycle however long it goes on (founder ruling, 2026-08-25).
#[test]
fn attachment_churn_spends_its_own_budget_and_never_the_daemon() {
    let (observations, observed) = observe_runs();
    let run = RunId::mint();

    for _ in 0..OBSERVATION_QUEUE * 4 {
        observations.report(RunOccurrence::Attached {
            run,
            at: std::time::SystemTime::UNIX_EPOCH,
        });
        observations.report(RunOccurrence::Detached {
            run,
            at: std::time::SystemTime::UNIX_EPOCH,
        });
    }

    assert_eq!(
        observations.integrity(),
        Integrity::Intact,
        "churn is noise, not a hole in the accounting"
    );
    drop(observed);
}

/// And it may not starve one either: the room an ending needs is not the room
/// churn is allowed to take.
#[test]
fn churn_leaves_room_for_the_ending_it_cannot_displace() {
    let (observations, observed) = observe_runs();
    for _ in 0..OBSERVATION_QUEUE * 4 {
        observations.report(RunOccurrence::Attached {
            run: RunId::mint(),
            at: std::time::SystemTime::UNIX_EPOCH,
        });
    }

    // Every remaining slot, taken by the facts the queue is reserved for.
    for _ in 0..(OBSERVATION_QUEUE - ADVISORY_SHARE) {
        observations.report(exited(RunId::mint()));
    }

    assert_eq!(
        observations.integrity(),
        Integrity::Intact,
        "churn took {ADVISORY_SHARE} slots and left the rest"
    );
    drop(observed);
}

/// A shutdown waits for what it must account for, not for advisory noise.
#[test]
fn settling_does_not_wait_on_attachment_activity() {
    let (observations, observed) = observe_runs();
    observations.report(RunOccurrence::Attached {
        run: RunId::mint(),
        at: std::time::SystemTime::UNIX_EPOCH,
    });

    assert_eq!(
        observations.settle(std::time::Duration::from_millis(50)),
        Integrity::Intact,
        "an attachment nobody drained is a line of history, not a hole"
    );
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
