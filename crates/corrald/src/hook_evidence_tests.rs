use super::*;

/// The queue is bounded and never waits. A caller that could block here is a
/// caller that can delay the user's agent (ADR 0004 D4).
#[test]
fn a_full_queue_drops_rather_than_waits() {
    let (deliveries, _receiver) = queue();
    let offered = std::time::Instant::now();
    for _ in 0..QUEUE * 2 {
        deliveries.offer(delivered());
    }
    assert!(
        offered.elapsed() < std::time::Duration::from_secs(1),
        "offering never waits on a drainer",
    );
}

/// A drainer that is gone is the same answer as a queue that is full: the
/// event is lost by design, and nothing on the endpoint's path notices.
#[test]
fn offering_to_a_closed_queue_is_quiet() {
    let (deliveries, receiver) = queue();
    drop(receiver);
    deliveries.offer(delivered());
}

/// A closed queue is not proof the daemon is leaving.
///
/// The receiver is also gone when the ingest task ended for any other reason,
/// and this thread cannot tell those apart. Reporting the ending as taken
/// would leave every later token resolving for the rest of a daemon that is
/// still serving — the state `forget_run` exists to prevent — so the caller is
/// told to retire it itself, which costs nothing if the daemon really is on
/// its way out.
#[test]
fn a_closed_queue_hands_the_retirement_back_rather_than_claiming_it() {
    let (deliveries, receiver) = queue();
    drop(receiver);

    // From a thread, because that is where its one caller lives.
    let announcing = deliveries.clone();
    let announced = std::thread::spawn(move || announcing.run_ended(corral_core::RunId::mint()))
        .join()
        .expect("the announcement returned");

    assert_eq!(
        announced,
        Retirement::QueueGone,
        "a closed queue was reported as having taken it",
    );
}

/// A queue that will not take the announcement inside its bound says so,
/// because the caller has to retire the token itself when it does.
///
/// A token that outlives its Run is how a provider process that outlived its
/// episode comes to contest the identity of the Run that replaced it — and
/// contested is monotonic with nothing in M1 to clear it (ADR 0004 D5, D8).
/// Out of order costs that Run's still-queued tail; never costs the Session.
#[test]
fn an_announcement_that_will_not_fit_says_so_rather_than_waiting_forever() {
    let (deliveries, _receiver) = queue_waiting(std::time::Duration::from_millis(30));
    for _ in 0..QUEUE {
        deliveries.offer(delivered());
    }

    let announcing = deliveries.clone();
    let announced = std::thread::spawn(move || announcing.run_ended(corral_core::RunId::mint()))
        .join()
        .expect("the announcement returned");

    assert_eq!(
        announced,
        Retirement::QueueFull,
        "a jammed queue was reported as having taken it",
    );
}

/// A Run's ending rides the same queue as that Run's events, so it lands
/// behind them. Retiring the token from another thread would race the events
/// already waiting — a session's last `SessionEnd` is delivered milliseconds
/// before its process exits — and would lose the tail of every session.
#[tokio::test]
async fn a_run_ending_arrives_behind_the_events_that_run_delivered() {
    let (deliveries, mut incoming) = queue();
    let run = corral_core::RunId::mint();

    deliveries.offer(delivered());
    // Announced the way the run lifecycle recorder announces it: from a thread
    // of its own, never from the reactor.
    let announcing = deliveries.clone();
    assert_eq!(
        std::thread::spawn(move || announcing.run_ended(run))
            .join()
            .expect("the announcement returned"),
        Retirement::Taken,
    );
    deliveries.offer(delivered());

    assert!(matches!(incoming.recv().await, Some(Ingest::Delivered(_))));
    assert!(matches!(
        incoming.recv().await,
        Some(Ingest::RunEnded(named)) if named == run
    ));
    assert!(matches!(incoming.recv().await, Some(Ingest::Delivered(_))));
}

fn delivered() -> Delivered {
    Delivered {
        scope: crate::hook_endpoint::DeliveryScope::Managed(
            crate::provider::LaunchTokens::new()
                .mint(crate::provider::LaunchScope {
                    session: corral_core::CorralSessionId::mint(),
                    run: corral_core::RunId::mint(),
                    provider: crate::provider::KnownProvider::Claude,
                })
                .expect("a token"),
        ),
        provider: "claude".to_owned(),
        payload: Some("{}".to_owned()),
        payload_omitted: None,
        observed_at: SystemTime::UNIX_EPOCH,
        arrived: std::time::Instant::now(),
    }
}

/// A managed session's global entry fires alongside its injected one —
/// measured 2026-09-02, milliseconds apart and in an unstable order. The
/// runtime is the daemon's own, so it is the launch that attributes it and
/// never discovery: whichever entry is taken in first, the identity belongs
/// to the managed Session, there is one Session and one Run, and no row is
/// shown for a runtime outside Corral.
#[tokio::test]
async fn a_managed_runtimes_global_entry_never_mints_a_session_whichever_arrives_first() {
    for (name, global_first) in [("global-first", true), ("injected-first", false)] {
        let registry = registry(name);
        let child = crate::runtime::spawn(
            &crate::runtime::LaunchRequest::new(
                "/bin/sh",
                ["-c", "sleep 30"].map(std::ffi::OsString::from),
                std::env::temp_dir(),
            )
            .expect("a launch request"),
            crate::runtime::PtyGeometry::expect_valid(24, 80),
        )
        .expect("a real child");
        let pid = child.process_id().expect("the child's pid");
        registry
            .state
            .with_runtime(|runtime| runtime.owned.register(child.owned()))
            .expect("the runtime");
        let scope = LaunchScope {
            session: corral_core::CorralSessionId::mint(),
            run: RunId::mint(),
            provider: KnownProvider::Claude,
        };
        registry
            .state
            .start_managed_session(
                command(name),
                scope.session,
                corral_state::LaunchedRun {
                    run: scope.run,
                    started: corral_core::OccurrenceTime::Authoritative(at(500)),
                    working_directory: std::path::PathBuf::from("/w"),
                },
                at(500),
            )
            .await
            .expect("the managed session");
        let identity = ExternalId::new("session-abc").expect("an identity");
        let global = || {
            crate::external_session::discovered(
                &registry.state,
                KnownProvider::Claude,
                identity.clone(),
                crate::ancestry::Corroboration::Reached {
                    provider: KnownProvider::Claude,
                    process: Box::new(crate::platform::process::ProcessIdentity {
                        pid,
                        parent: std::process::id(),
                        group: pid,
                        started: at(500),
                        executable: std::path::PathBuf::from("/usr/local/bin/claude"),
                    }),
                },
                observed(900),
                None,
            )
        };
        let injected = || establish(&registry.state, &scope, identity.clone(), at(900));
        if global_first {
            assert_eq!(global().await.expect("recorded"), None, "{name}");
            injected().await.expect("recorded");
        } else {
            injected().await.expect("recorded");
            assert_eq!(global().await.expect("recorded"), None, "{name}");
        }

        let sessions = registry.state.sessions().await.expect("sessions");
        assert_eq!(sessions.len(), 1, "{name}: a second session was minted");
        assert_eq!(sessions[0].id(), scope.session, "{name}");
        let binding = registry
            .state
            .provider_session_binding(scope.session)
            .await
            .expect("bindings")
            .unwrap_or_else(|| panic!("{name}: the managed session was refused its identity"));
        assert_eq!(binding.key().external_id(), &identity, "{name}");
        let runs = registry.state.runs_of(scope.session).await.expect("runs");
        assert_eq!(runs.len(), 1, "{name}");
        assert!(
            registry.state.seen_runtimes().snapshot().is_empty(),
            "{name}: the managed runtime was shown as one outside Corral",
        );

        if let Some(group) = child.child_group() {
            group.hang_up();
        }
        let (_screen, mut reaper) = child.split();
        let _ = reaper.wait();
    }
}

struct Registry {
    state: Arc<DaemonState>,
    directory: std::path::PathBuf,
}

impl Drop for Registry {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn registry(name: &str) -> Registry {
    let directory = std::env::temp_dir().join(format!(
        "corrald-hook-evidence-{}-{name}",
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

/// The same moment as the daemon observes it: both clocks, advanced together.
fn observed(seconds: u64) -> crate::clock::Reading {
    crate::clock::Reading {
        mono: crate::clock::Monotonic::from_millis(seconds * 1_000),
        wall: at(seconds),
    }
}

fn command(id: &str) -> corral_core::Command {
    corral_core::Command::new(
        corral_core::CommandId::new(id).expect("usable"),
        corral_core::CommandFingerprint::builder(
            corral_core::CommandKind::new("session.new").expect("usable"),
        )
        .input("cwd", "/tmp")
        .build(),
    )
}
