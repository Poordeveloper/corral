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

    assert!(!announced, "a closed queue was reported as having taken it");
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

    assert!(!announced, "a jammed queue was reported as having taken it");
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
    assert!(
        std::thread::spawn(move || announcing.run_ended(run))
            .join()
            .expect("the announcement returned"),
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
        token: crate::provider::LaunchTokens::new()
            .mint(crate::provider::LaunchScope {
                session: corral_core::CorralSessionId::mint(),
                run: corral_core::RunId::mint(),
                provider: crate::provider::KnownProvider::Claude,
            })
            .expect("a token"),
        provider: "claude".to_owned(),
        payload: Some("{}".to_owned()),
        payload_omitted: None,
        observed_at: SystemTime::UNIX_EPOCH,
        arrived: std::time::Instant::now(),
    }
}
