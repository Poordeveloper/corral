use super::*;

use std::path::PathBuf;
use std::time::Duration;

fn at(seconds: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
}

fn candidate(pid: u32, started: u64, provider: KnownProvider) -> RuntimeCandidate {
    RuntimeCandidate {
        provider,
        process: ProcessIdentity {
            pid,
            parent: 1,
            group: pid,
            started: at(started),
            executable: PathBuf::from("/usr/local/bin/claude"),
        },
        provisional_id: corral_core::CorralSessionId::mint(),
        identified: None,
    }
}

fn identified(session: CorralSessionId) -> Identified {
    Identified {
        session,
        external_id: ExternalId::new("session-abc").expect("an identity"),
        run: RunId::mint(),
    }
}

/// A pass that read the table and could inspect every pid on it.
fn read(found: Vec<RuntimeCandidate>) -> Pass {
    Pass::Read {
        found,
        uninspected: HashSet::new(),
    }
}

#[test]
fn a_first_pass_reports_everything_it_found_as_new() {
    let mut seen = SeenRuntimes::new();

    let changes = seen.absorb(read(vec![
        candidate(10, 100, KnownProvider::Claude),
        candidate(20, 200, KnownProvider::Codex),
    ]));

    assert_eq!(changes.appeared.len(), 2);
    assert!(changes.gone.is_empty());
    assert_eq!(seen.all().count(), 2);
}

/// A runtime does not become newer by being looked at again, so a second pass
/// that finds the same process reports nothing at all.
#[test]
fn seeing_the_same_runtime_again_changes_nothing() {
    let mut seen = SeenRuntimes::new();
    seen.absorb(read(vec![candidate(10, 100, KnownProvider::Claude)]));

    let changes = seen.absorb(read(vec![candidate(10, 100, KnownProvider::Claude)]));

    assert_eq!(changes, Changes::default());
}

#[test]
fn a_runtime_that_is_no_longer_there_is_reported_gone() {
    let mut seen = SeenRuntimes::new();
    seen.absorb(read(vec![
        candidate(10, 100, KnownProvider::Claude),
        candidate(20, 200, KnownProvider::Codex),
    ]));

    let changes = seen.absorb(read(vec![candidate(10, 100, KnownProvider::Claude)]));

    assert!(changes.appeared.is_empty());
    assert_eq!(changes.gone.len(), 1);
    assert_eq!(changes.gone[0].process().pid, 20);
    assert_eq!(seen.all().count(), 1);
}

/// The pid was reused. The process there now is a different runtime, so one
/// appeared and one went — never one that quietly became the other.
#[test]
fn a_reused_pid_is_a_new_runtime_and_the_old_one_is_gone() {
    let mut seen = SeenRuntimes::new();
    seen.absorb(read(vec![candidate(10, 100, KnownProvider::Claude)]));

    let changes = seen.absorb(read(vec![candidate(10, 900, KnownProvider::Claude)]));

    assert_eq!(changes.appeared.len(), 1);
    assert_eq!(changes.appeared[0].process().started, at(900));
    assert_eq!(changes.gone.len(), 1);
    assert_eq!(changes.gone[0].process().started, at(100));
    assert_eq!(seen.all().count(), 1);
}

/// "I could not look" is not evidence that anything stopped. A pass that
/// cannot enumerate retires nothing and leaves the table exactly as it was —
/// otherwise one failed read would end every runtime Corral had found.
#[test]
fn a_pass_that_cannot_read_the_table_retires_nothing() {
    let mut seen = SeenRuntimes::new();
    seen.absorb(read(vec![candidate(10, 100, KnownProvider::Claude)]));

    let changes = seen.absorb(Pass::Unavailable);

    assert_eq!(changes, Changes::default());
    assert_eq!(seen.all().count(), 1);
}

/// The same rule for one process as for the whole table. A pid the listing
/// names but this account may not inspect is a process that is still there,
/// so the runtime held under it is neither retired nor reported gone —
/// until a pass sees the pid absent, which is the positive answer.
#[test]
fn a_runtime_whose_pid_cannot_be_inspected_is_kept_until_it_is_seen_gone() {
    let mut seen = SeenRuntimes::new();
    seen.absorb(read(vec![candidate(10, 100, KnownProvider::Claude)]));

    let changes = seen.absorb(Pass::Read {
        found: Vec::new(),
        uninspected: HashSet::from([10]),
    });

    assert_eq!(changes, Changes::default());
    assert_eq!(seen.all().count(), 1);

    let changes = seen.absorb(read(Vec::new()));

    assert_eq!(changes.gone.len(), 1);
    assert_eq!(changes.gone[0].process().pid, 10);
    assert_eq!(seen.all().count(), 0);
}

/// On a platform this build cannot observe, a pass says so rather than
/// reporting an empty machine.
#[cfg(target_os = "macos")]
#[test]
fn a_pass_on_an_unobservable_platform_is_unavailable() {
    assert_eq!(once(), Pass::Unavailable);
}

/// On a platform that can read the table, a pass reads it — and finds no
/// provider here, because a test binary is not one. The claim under test is
/// that recognition is applied at all, not that this machine runs an agent.
#[cfg(target_os = "linux")]
#[test]
fn a_pass_reads_the_table_and_recognizes_only_providers() {
    let Pass::Read { found, .. } = once() else {
        panic!("this platform can read the process table");
    };

    for candidate in found {
        assert!(
            crate::provider::recognition::provider_of(&candidate.process().executable).is_some(),
            "the sweep reported a process it does not recognize: {:?}",
            candidate.process().executable,
        );
    }
}

#[test]
fn the_shared_table_survives_a_poisoned_holder() {
    let shared = SharedSeenRuntimes::new();
    let poisoner = shared.clone();
    let _ = std::thread::spawn(move || {
        poisoner.absorb(read(vec![candidate(10, 100, KnownProvider::Claude)]));
        panic!("a holder panicked mid-use");
    })
    .join();

    shared.absorb(read(vec![
        candidate(10, 100, KnownProvider::Claude),
        candidate(20, 200, KnownProvider::Codex),
    ]));

    assert_eq!(shared.snapshot().len(), 2);
}

/// Discovery names the Session a seen runtime is carrying, and the next pass
/// — which mints a fresh, unidentified candidate for the same process, as a
/// real pass does — keeps what discovery said rather than forgetting it.
#[test]
fn an_identified_runtime_keeps_its_identity_across_passes() {
    let mut seen = SeenRuntimes::new();
    let process = candidate(10, 100, KnownProvider::Claude);
    let session = CorralSessionId::mint();
    seen.absorb(read(vec![process.clone()]));

    seen.identify(
        KnownProvider::Claude,
        process.process(),
        identified(session),
    );
    let changes = seen.absorb(read(vec![candidate(10, 100, KnownProvider::Claude)]));

    assert_eq!(changes, Changes::default());
    let held = seen.all().next().expect("the runtime");
    assert_eq!(held.identified().map(|it| it.session), Some(session));
}

/// A delivery can arrive before any pass has seen the process. The row must
/// not wait a cadence for identity the daemon already holds, so discovery
/// puts the runtime on the table itself — and the pass that then finds it
/// reports nothing new.
#[test]
fn a_delivery_before_any_pass_puts_the_runtime_on_the_table_identified() {
    let mut seen = SeenRuntimes::new();
    let process = candidate(10, 100, KnownProvider::Claude);
    let session = CorralSessionId::mint();

    seen.identify(
        KnownProvider::Claude,
        process.process(),
        identified(session),
    );

    let held = seen.all().next().expect("the runtime");
    assert_eq!(held.identified().map(|it| it.session), Some(session));
    assert_eq!(held.provider(), KnownProvider::Claude);
    let changes = seen.absorb(read(vec![candidate(10, 100, KnownProvider::Claude)]));
    assert_eq!(changes, Changes::default());
}

/// What went is reported with what was known about it, so the Run an
/// identified runtime was in can be ended by whoever reads the change.
#[test]
fn an_identified_runtime_seen_gone_is_reported_gone_with_its_identity() {
    let mut seen = SeenRuntimes::new();
    let process = candidate(10, 100, KnownProvider::Claude);
    let session = CorralSessionId::mint();
    seen.absorb(read(vec![process.clone()]));
    seen.identify(
        KnownProvider::Claude,
        process.process(),
        identified(session),
    );

    let changes = seen.absorb(read(Vec::new()));

    assert_eq!(changes.gone.len(), 1);
    assert_eq!(
        changes.gone[0].identified().map(|it| it.session),
        Some(session)
    );
    assert_eq!(seen.all().count(), 0);
}

/// A row must not change identity under the user between passes: the same
/// runtime keeps the identity it was first shown under.
#[test]
fn a_runtime_keeps_its_row_identity_across_passes() {
    let mut seen = SeenRuntimes::new();
    let first = candidate(10, 100, KnownProvider::Claude);
    let shown_as = first.provisional_id();
    seen.absorb(read(vec![first]));

    // A later pass mints a fresh candidate for the same process, as a real
    // pass does.
    seen.absorb(read(vec![candidate(10, 100, KnownProvider::Claude)]));

    assert_eq!(
        seen.all().next().expect("the runtime").provisional_id(),
        shown_as,
    );
}

/// A pass keeps only what is outside Corral. The daemon's own child is on the
/// same table as every other provider process, and so is what it spawned —
/// a launcher's native child runs in the launcher's group — while a provider
/// in some other group is somebody else's however it is parented.
#[test]
fn a_pass_keeps_only_the_runtimes_outside_corral() {
    let corrals_child = 500;
    let mut own = candidate(corrals_child, 100, KnownProvider::Codex);
    own.process.group = corrals_child;
    let mut spawned_by_it = candidate(501, 101, KnownProvider::Codex);
    spawned_by_it.process.parent = corrals_child;
    spawned_by_it.process.group = corrals_child;
    let mut somebody_elses = candidate(600, 100, KnownProvider::Claude);
    somebody_elses.process.parent = corrals_child;
    somebody_elses.process.group = 600;

    let pass = read(vec![own, spawned_by_it, somebody_elses.clone()])
        .outside(&HashSet::from([corrals_child]));

    assert_eq!(
        pass,
        read(vec![somebody_elses]),
        "the daemon's own group was not left out",
    );
    assert_eq!(
        Pass::Unavailable.outside(&HashSet::from([corrals_child])),
        Pass::Unavailable
    );
}
