use super::*;

const GRACE: Duration = Duration::from_secs(60);

#[test]
fn a_fresh_daemon_is_already_counting_down() {
    let start = Instant::now();
    let lifecycle = Lifecycle::new(start);

    assert!(matches!(
        lifecycle.poll_idle(GRACE, start),
        IdleCheck::Wait(_)
    ));
}

#[test]
fn an_established_client_stops_the_countdown() {
    let start = Instant::now();
    let lifecycle = Lifecycle::new(start);

    let guard = lifecycle.establish().expect("running");

    assert_eq!(lifecycle.poll_idle(GRACE, start + GRACE), IdleCheck::Busy);
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
        lifecycle.poll_idle(GRACE, start + GRACE),
        IdleCheck::Wait(_)
    ));
}

#[test]
fn establishing_before_the_commit_prevents_idle_shutdown() {
    let start = Instant::now();
    let lifecycle = Lifecycle::new(start);

    let _guard = lifecycle.establish().expect("running");

    assert_eq!(lifecycle.poll_idle(GRACE, start + GRACE), IdleCheck::Busy);
    assert_eq!(lifecycle.phase(), Phase::Running);
}

#[test]
fn establishing_after_the_commit_is_refused() {
    let start = Instant::now();
    let lifecycle = Lifecycle::new(start);

    assert_eq!(
        lifecycle.poll_idle(GRACE, start + GRACE),
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
        lifecycle.poll_idle(GRACE, start + GRACE),
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

/// Whichever cause commits the shutdown first wins the reason, so the exit
/// status cannot be read off it: a signal landing in the same moment as the
/// store's conclusion would report a clean stop from a daemon that could not
/// trust its own durable state.
#[test]
fn an_untrusted_store_decides_the_exit_whatever_committed_the_shutdown() {
    let lifecycle = Lifecycle::new(Instant::now());
    assert_eq!(lifecycle.exit_disposition(), ExitDisposition::Clean);

    lifecycle.commit_shutdown(ShutdownReason::Signal("SIGTERM"));
    lifecycle.note_untrusted_state();

    assert_eq!(
        lifecycle.shutdown_reason(),
        Some(ShutdownReason::Signal("SIGTERM")),
        "the first cause still owns the reason"
    );
    assert_eq!(
        lifecycle.exit_disposition(),
        ExitDisposition::UntrustedState
    );
}

#[test]
fn a_clean_shutdown_stays_a_clean_exit() {
    let lifecycle = Lifecycle::new(Instant::now());

    lifecycle.commit_shutdown(ShutdownReason::Signal("SIGINT"));

    assert_eq!(lifecycle.exit_disposition(), ExitDisposition::Clean);
}
