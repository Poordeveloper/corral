use super::*;

const GRACE: Duration = Duration::from_secs(60);

#[test]
fn a_fresh_daemon_is_already_counting_down() {
    let start = Instant::now();
    let lifecycle = Lifecycle::new(start);

    assert!(matches!(
        lifecycle.poll_idle(GRACE, start, 0),
        IdleCheck::Wait(_)
    ));
}

#[test]
fn an_established_client_stops_the_countdown() {
    let start = Instant::now();
    let lifecycle = Lifecycle::new(start);

    let guard = lifecycle.establish().expect("running");

    assert_eq!(
        lifecycle.poll_idle(GRACE, start + GRACE, 0),
        IdleCheck::Busy
    );
    drop(guard);
}

#[test]
fn the_countdown_restarts_when_the_last_client_leaves() {
    let start = Instant::now();
    let lifecycle = Lifecycle::new(start);

    let guard = lifecycle.establish().expect("running");
    drop(guard);

    // The grace that elapsed before the client connected does not count.
    assert!(matches!(
        lifecycle.poll_idle(GRACE, start + GRACE, 0),
        IdleCheck::Wait(_)
    ));
}

#[test]
fn establishing_before_the_commit_prevents_idle_shutdown() {
    let start = Instant::now();
    let lifecycle = Lifecycle::new(start);

    let _guard = lifecycle.establish().expect("running");

    assert_eq!(
        lifecycle.poll_idle(GRACE, start + GRACE, 0),
        IdleCheck::Busy
    );
    assert_eq!(lifecycle.phase(), Phase::Running);
}

#[test]
fn establishing_after_the_commit_is_refused() {
    let start = Instant::now();
    let lifecycle = Lifecycle::new(start);

    assert_eq!(
        lifecycle.poll_idle(GRACE, start + GRACE, 0),
        IdleCheck::Committed
    );

    assert!(lifecycle.establish().is_none());
    assert_eq!(lifecycle.phase(), Phase::ShuttingDown);
}

#[test]
fn a_committed_shutdown_is_never_taken_twice() {
    let start = Instant::now();
    let lifecycle = Lifecycle::new(start);

    assert!(lifecycle.commit_shutdown(ShutdownReason::Signal("SIGTERM")));
    assert!(!lifecycle.commit_shutdown(ShutdownReason::Signal("SIGINT")));
    assert_eq!(
        lifecycle.poll_idle(GRACE, start + GRACE, 0),
        IdleCheck::AlreadyCommitted
    );
    assert_eq!(
        lifecycle.shutdown_reason(),
        Some(ShutdownReason::Signal("SIGTERM"))
    );
}

#[test]
fn a_signal_shuts_down_with_clients_still_established() {
    let start = Instant::now();
    let lifecycle = Lifecycle::new(start);
    let _guard = lifecycle.establish().expect("running");

    assert!(lifecycle.commit_shutdown(ShutdownReason::Signal("SIGTERM")));
    assert_eq!(lifecycle.phase(), Phase::ShuttingDown);
}

#[tokio::test]
async fn shutdown_is_broadcast_to_every_subscriber() {
    let lifecycle = Lifecycle::new(Instant::now());
    let mut first = lifecycle.subscribe();
    let mut second = lifecycle.subscribe();

    lifecycle.commit_shutdown(ShutdownReason::Idle);

    assert!(first.changed().await.is_ok());
    assert!(second.changed().await.is_ok());
    assert!(*first.borrow());
}

/// Managed work holds the daemon exactly as a client does. Without this a
/// daemon exits sixty seconds after the last person detaches and hangs up every
/// agent it was asked to keep.
#[test]
fn a_live_run_holds_the_daemon_open_with_no_clients() {
    let started = Instant::now();
    let lifecycle = Lifecycle::new(started);

    let verdict = lifecycle.poll_idle(
        Duration::from_millis(1),
        started + Duration::from_secs(3600),
        1,
    );

    assert_eq!(
        verdict,
        IdleCheck::Busy,
        "the daemon was ready to exit under managed work"
    );
    assert_eq!(lifecycle.managed_sessions(), 1);
}

/// The last run ending starts the clock, the same way the last client leaving
/// does — and it is observed at the check rather than announced, so nobody can
/// forget to report it.
#[test]
fn the_last_run_ending_makes_the_daemon_idle() {
    let started = Instant::now();
    let lifecycle = Lifecycle::new(started);
    assert_eq!(lifecycle.poll_idle(GRACE, started, 2), IdleCheck::Busy);

    let verdict = lifecycle.poll_idle(GRACE, started, 0);

    assert!(matches!(verdict, IdleCheck::Wait(_)));
    assert_eq!(
        lifecycle.poll_idle(
            Duration::from_millis(1),
            started + Duration::from_secs(3600),
            0
        ),
        IdleCheck::Committed
    );
}

/// A finished run does not hold the daemon: only live ones do, which is what
/// keeps a daemon that ran one command from staying awake for the machine's
/// uptime.
#[test]
fn a_finished_run_stops_holding_the_daemon() {
    let started = Instant::now();
    let lifecycle = Lifecycle::new(started);
    let _client = lifecycle.establish().expect("established");

    // One session in the registry, none of them running.
    let verdict = lifecycle.poll_idle(GRACE, started, 0);

    assert_eq!(
        verdict,
        IdleCheck::Busy,
        "a client still holds it, which is a different reason"
    );
    assert_eq!(lifecycle.managed_sessions(), 0);
}

/// A client and a live run each hold it: neither leaving alone is enough.
#[test]
fn a_client_and_a_run_hold_the_daemon_independently() {
    let started = Instant::now();
    let lifecycle = Lifecycle::new(started);
    let client = lifecycle.establish().expect("established");

    drop(client);

    assert_eq!(
        lifecycle.poll_idle(
            Duration::from_millis(1),
            started + Duration::from_secs(3600),
            1
        ),
        IdleCheck::Busy,
        "a live run stopped holding the daemon when a client left"
    );
}
