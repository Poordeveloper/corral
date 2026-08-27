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
