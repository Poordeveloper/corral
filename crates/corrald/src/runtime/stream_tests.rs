use super::*;

#[test]
fn a_stream_starts_at_the_first_epoch_and_sequence() {
    let stream = TerminalStream::new();

    assert_eq!(stream.epoch(), Epoch(0));
    assert_eq!(stream.next_sequence(), Sequence(0));
}

#[test]
fn output_takes_successive_sequences() {
    let mut stream = TerminalStream::new();

    assert_eq!(stream.advance(), Sequence(0));
    assert_eq!(stream.advance(), Sequence(1));
    assert_eq!(stream.next_sequence(), Sequence(2));
}

/// A sequence only means anything within the screen shape it was recorded
/// against, so a reflow restarts both.
#[test]
fn a_reflow_opens_a_new_epoch_and_restarts_the_sequence() {
    let mut stream = TerminalStream::new();
    stream.advance();
    stream.advance();

    let epoch = stream.open_epoch();

    assert_eq!(epoch, Epoch(1));
    assert_eq!(stream.next_sequence(), Sequence(0));
}

/// Each viewer gets its own snapshot and joins the same stream; neither owns
/// the terminal (grill Q6).
#[test]
fn two_viewers_join_the_same_stream_independently() {
    let mut stream = TerminalStream::new();
    stream.advance();
    let mut first = stream.subscriber();
    let mut second = stream.subscriber();

    assert_eq!(first.queue(b"output").expect("queued"), Sequence(1));
    assert_eq!(second.queue(b"output").expect("queued"), Sequence(1));
    assert_eq!(first.take_queued().as_deref(), Some(b"output".as_slice()));
    assert_eq!(
        second.queued_bytes(),
        b"output".len(),
        "one viewer's read drained another's queue"
    );
}

/// Bytes from a screen shape that no longer exists must not be replayed into a
/// reflowed replica — the divergence the epoch exists to prevent.
#[test]
fn a_subscriber_rejects_frames_from_an_epoch_it_has_left() {
    let mut stream = TerminalStream::new();
    let mut subscriber = stream.subscriber();
    let stale = stream.epoch();

    let fresh = stream.open_epoch();
    subscriber.enter_epoch(fresh);

    assert!(subscriber.accepts(fresh));
    assert!(!subscriber.accepts(stale));
}

#[test]
fn entering_an_epoch_discards_what_belonged_to_the_old_one() {
    let mut stream = TerminalStream::new();
    let mut subscriber = stream.subscriber();
    subscriber.queue(b"pre-resize output").expect("queued");

    subscriber.enter_epoch(stream.open_epoch());

    assert_eq!(subscriber.queued_bytes(), 0);
    assert_eq!(subscriber.take_queued(), None);
}

/// A viewer missing bytes out of the middle would render a screen that looks
/// plausible and is wrong. Losing the whole queue and resyncing is the visible,
/// honest failure.
#[test]
fn an_overflowing_subscriber_loses_its_stream_rather_than_its_middle() {
    let stream = TerminalStream::new();
    let mut subscriber = stream.subscriber();
    let chunk = vec![b'x'; 1024 * 1024];

    let mut overflowed = false;
    for _ in 0..8 {
        if subscriber.queue(&chunk).is_err() {
            overflowed = true;
            break;
        }
    }

    assert!(overflowed, "the queue never enforced its budget");
    assert_eq!(
        subscriber.desynchronised(),
        Some(Desynchronised::QueueOverflow)
    );
    assert_eq!(
        subscriber.queued_bytes(),
        0,
        "a desynchronised subscriber kept a partial stream"
    );
    assert_eq!(subscriber.take_queued(), None);
}

/// One stalled viewer must not shrink what another may buffer, and must not
/// slow the stream itself.
#[test]
fn one_desynchronised_viewer_leaves_the_others_untouched() {
    let mut stream = TerminalStream::new();
    let mut stalled = stream.subscriber();
    let mut healthy = stream.subscriber();
    let chunk = vec![b'x'; SUBSCRIBER_QUEUE_BYTES + 1];

    assert!(stalled.queue(&chunk).is_err());

    assert!(healthy.queue(b"still flowing").is_ok());
    assert_eq!(healthy.desynchronised(), None);
    assert_eq!(stream.advance(), Sequence(0), "the stream itself stalled");
}

#[test]
fn a_desynchronised_subscriber_stays_refused_until_it_resyncs() {
    let stream = TerminalStream::new();
    let mut subscriber = stream.subscriber();
    let chunk = vec![b'x'; SUBSCRIBER_QUEUE_BYTES + 1];
    assert!(subscriber.queue(&chunk).is_err());

    assert_eq!(
        subscriber.queue(b"anything"),
        Err(Desynchronised::QueueOverflow)
    );

    subscriber.enter_epoch(stream.epoch());
    assert!(
        subscriber.queue(b"after a resync").is_ok(),
        "a resynced subscriber never recovered"
    );
}
