use super::*;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A daemon state on a real registry file, because what these tests assert is
/// what the durable log holds afterwards.
struct Registry {
    state: Arc<DaemonState>,
    directory: PathBuf,
}

impl Drop for Registry {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn registry(name: &str) -> Registry {
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "corrald-external-{}-{unique}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("create the scratch directory");
    let state = DaemonState::open(
        &directory.join("registry.sqlite3"),
        &directory.join("launch"),
        &directory,
    )
    .expect("open");
    Registry {
        state: Arc::new(state),
        directory,
    }
}

fn at(seconds: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
}

fn identity(raw: &str) -> ExternalId {
    ExternalId::new(raw).expect("a usable identity")
}

fn reached(pid: u32, started: SystemTime) -> Corroboration {
    Corroboration::Reached {
        provider: KnownProvider::Claude,
        process: Box::new(ProcessIdentity {
            pid,
            parent: 1,
            started,
            executable: PathBuf::from("/usr/local/bin/claude"),
        }),
    }
}

/// The whole point of the corroborated path: a session Corral never started
/// becomes one Corral can see, with the Run the runtime is already in.
#[tokio::test]
async fn a_corroborated_delivery_makes_a_session_visible() {
    let registry = registry("discovers");

    let outcome = discovered(
        &registry.state,
        KnownProvider::Claude,
        identity("session-abc"),
        reached(4321, at(500)),
        at(900),
    )
    .await
    .expect("recorded");

    assert_eq!(outcome, Some(Discovered::Session));
}

/// The Run's start is the runtime's own. A process that began before this
/// daemon existed still began when it began, and the moment Corral looked is
/// never written as a start time.
#[tokio::test]
async fn the_run_starts_when_the_runtime_started_not_when_corral_looked() {
    let registry = registry("run-start");
    discovered(
        &registry.state,
        KnownProvider::Claude,
        identity("session-abc"),
        reached(4321, at(500)),
        at(900),
    )
    .await
    .expect("recorded");

    let sessions = registry.state.sessions().await.expect("sessions");
    let session = sessions.first().expect("the discovered session").id();
    let runs = registry.state.runs_of(session).await.expect("runs");

    assert_eq!(runs.len(), 1);
    assert_eq!(
        runs[0].started_at(),
        corral_core::OccurrenceTime::Authoritative(at(500)),
    );
}

/// Payload identity alone proves a provider thread emitted an event. It does
/// not prove a user's session exists, and one measured Codex turn emits a
/// second identity for the provider's own internal work — so an
/// uncorroborated delivery mints nothing at all (grill Q6′).
#[tokio::test]
async fn an_uncorroborated_delivery_mints_nothing() {
    let registry = registry("uncorroborated");

    for corroboration in [Corroboration::NotFound, Corroboration::Unreadable] {
        let outcome = discovered(
            &registry.state,
            KnownProvider::Codex,
            identity("thread-title-generation"),
            corroboration,
            at(900),
        )
        .await
        .expect("recorded");

        assert_eq!(outcome, None);
    }

    assert!(
        registry
            .state
            .sessions()
            .await
            .expect("sessions")
            .is_empty()
    );
}

/// One event, two channels, one fact. A managed session's global entry fires
/// alongside its injected one — measured 2026-09-02, milliseconds apart and
/// in an unstable order — and the second must never make a second Session or
/// a second Run.
#[tokio::test]
async fn a_second_delivery_of_one_identity_confirms_rather_than_duplicates() {
    let registry = registry("dedupe");
    discovered(
        &registry.state,
        KnownProvider::Claude,
        identity("session-abc"),
        reached(4321, at(500)),
        at(900),
    )
    .await
    .expect("recorded");

    let again = discovered(
        &registry.state,
        KnownProvider::Claude,
        identity("session-abc"),
        reached(4321, at(500)),
        at(901),
    )
    .await
    .expect("recorded");

    assert_eq!(again, Some(Discovered::AlreadyKnown));
    let sessions = registry.state.sessions().await.expect("sessions");
    assert_eq!(sessions.len(), 1);
    let runs = registry
        .state
        .runs_of(sessions[0].id())
        .await
        .expect("runs");
    assert_eq!(runs.len(), 1);
}

/// A known identity is not a completed discovery. The Session and its
/// provider binding commit before the runtime and its Run do, so a store
/// that was busy for the second half leaves a Session with no Run — and a
/// delivery that then merely "confirmed" the identity would leave it that
/// way for good. The next corroborated delivery records the Run.
#[tokio::test]
async fn a_session_left_without_a_run_gets_one_on_the_next_delivery() {
    let registry = registry("partial-discovery");
    // The first half of a discovery, exactly as `discovered` performs it.
    let key = BindingKey::new(
        registry.state.node(),
        BindingKind::ProviderSession,
        ProviderId::new("claude").expect("a provider"),
        identity("session-abc"),
    );
    registry
        .state
        .resolve_or_create_session(
            key,
            Provenance::Discovered,
            Evidence::new(EvidenceSource::ProviderHook, Assurance::Attested, at(900)),
            at(900),
        )
        .await
        .expect("the session and its identity");

    let outcome = discovered(
        &registry.state,
        KnownProvider::Claude,
        identity("session-abc"),
        reached(4321, at(500)),
        at(901),
    )
    .await
    .expect("recorded");

    assert_eq!(outcome, Some(Discovered::Run));
    let sessions = registry.state.sessions().await.expect("sessions");
    assert_eq!(sessions.len(), 1);
    let runs = registry
        .state
        .runs_of(sessions[0].id())
        .await
        .expect("runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(
        runs[0].started_at(),
        corral_core::OccurrenceTime::Authoritative(at(500)),
    );
}

/// The one store answer discovery tolerates is "this runtime is another
/// Session's" — succession, which ADR 0014 D7 rules on and this build does
/// not implement. Every other answer is the store's own and reaches the
/// caller: a store that cannot vouch for durable truth must not be read as
/// ordinary succession and logged past.
#[tokio::test]
async fn a_store_refusal_that_is_not_succession_reaches_the_caller() {
    let registry = registry("store-refusal");
    let nobody = corral_core::CorralSessionId::mint();

    let outcome = record_run(
        &registry.state,
        nobody,
        ProviderId::new("claude").expect("a provider"),
        &ProcessIdentity {
            pid: 4321,
            parent: 1,
            started: at(500),
            executable: PathBuf::from("/usr/local/bin/claude"),
        },
        at(900),
    )
    .await;

    assert!(
        matches!(
            outcome,
            Err(StateError::Refused(Refusal::UnknownSession(session))) if session == nobody
        ),
        "the store's refusal was swallowed",
    );
}

/// Two provider identities are two Sessions. Nothing correlates them by time
/// or by the process they share — heuristics never bind.
#[tokio::test]
async fn two_identities_are_two_sessions_even_from_one_process() {
    let registry = registry("two-identities");
    for name in ["session-one", "session-two"] {
        discovered(
            &registry.state,
            KnownProvider::Claude,
            identity(name),
            reached(4321, at(500)),
            at(900),
        )
        .await
        .expect("recorded");
    }

    assert_eq!(registry.state.sessions().await.expect("sessions").len(), 2);
}

/// A reused pid must never file a new process's Run under the Session of the
/// process that held the number before it. The start time is what makes the
/// name unique.
#[test]
fn one_pid_at_two_start_times_is_two_incarnations() {
    let first = ProcessIdentity {
        pid: 4321,
        parent: 1,
        started: at(500),
        executable: PathBuf::from("/usr/local/bin/claude"),
    };
    let reused = ProcessIdentity {
        started: at(900),
        ..first.clone()
    };

    assert_ne!(incarnation_of(&first), incarnation_of(&reused));
}

/// An external Session is read-only by structure. The binding that names its
/// runtime was discovered rather than created, and nothing about it may drive
/// control (ADR 0014 D6).
#[tokio::test]
async fn a_discovered_runtime_never_becomes_a_control_capable_binding() {
    let registry = registry("read-only");
    discovered(
        &registry.state,
        KnownProvider::Claude,
        identity("session-abc"),
        reached(4321, at(500)),
        at(900),
    )
    .await
    .expect("recorded");

    let session = registry.state.sessions().await.expect("sessions")[0].id();
    let bindings = registry.state.bindings_of(session).await.expect("bindings");
    let runtime = bindings
        .iter()
        .find(|binding| binding.key().kind() == corral_core::BindingKind::Runtime)
        .expect("a runtime binding");

    assert_eq!(runtime.provenance(), Provenance::Discovered);
    assert!(!runtime.is_control_capable_runtime_binding());
}

/// A daemon that restarts must not leave a Run it recorded before still shown
/// as running. Every external Run is re-verified, and one whose process this
/// build cannot observe ends `Unverifiable` rather than staying open
/// (ADR 0014 D5).
#[tokio::test]
async fn a_restart_resolves_every_external_run_it_recorded() {
    let registry = registry("reverify");
    discovered(
        &registry.state,
        KnownProvider::Claude,
        identity("session-abc"),
        reached(u32::MAX - 1, at(500)),
        at(900),
    )
    .await
    .expect("recorded");

    reverify_external_runs(Arc::clone(&registry.state)).await;

    let session = registry.state.sessions().await.expect("sessions")[0].id();
    let runs = registry.state.runs_of(session).await.expect("runs");
    assert!(
        runs[0].ended_at().is_some(),
        "the run was left open after re-verification",
    );
}

/// A process this account may not inspect is not a process that stopped.
/// Unknown is a first-class state and never collapses into exited.
#[tokio::test]
async fn a_run_that_cannot_be_verified_ends_unverifiable_rather_than_exited() {
    let observation = crate::platform::process::Observation::NotPermitted;

    assert_eq!(end_for(&observation, "pid-1-0"), RunEnd::Unverifiable);
    assert_eq!(
        end_for(
            &crate::platform::process::Observation::Unobservable,
            "pid-1-0"
        ),
        RunEnd::Unverifiable,
    );
    assert_eq!(
        end_for(&crate::platform::process::Observation::Gone, "pid-1-0"),
        RunEnd::Exited(corral_core::ExitCause::Unknown),
    );
}

/// The pid was reused. The process there now is not the one this Run named,
/// so the Run's process is gone — and the start time is what says so.
#[test]
fn a_reused_pid_is_not_the_process_the_run_named() {
    let now_running =
        crate::platform::process::Observation::Identified(Box::new(ProcessIdentity {
            pid: 4321,
            parent: 1,
            started: at(900),
            executable: PathBuf::from("/usr/bin/something-else"),
        }));

    assert_eq!(
        end_for(&now_running, "pid-4321-500000000"),
        RunEnd::Exited(corral_core::ExitCause::Unknown),
    );
}
