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
    // From a thread, because that is where its one caller lives.
    let announcing = deliveries.clone();
    std::thread::spawn(move || announcing.run_ended(corral_core::RunId::mint()))
        .join()
        .expect("the announcement returned");
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
    std::thread::spawn(move || announcing.run_ended(run))
        .join()
        .expect("the announcement returned");
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
    }
}
