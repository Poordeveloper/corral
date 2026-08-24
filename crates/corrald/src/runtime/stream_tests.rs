use super::*;

/// Drain a viewer, returning what it received.
///
/// Dropping each delivery is what returns its room, so this is also how a
/// healthy client's budget recovers.
fn drain(viewer: &mut Viewer) -> Vec<Vec<u8>> {
    let mut received = Vec::new();
    while let Ok(delivery) = viewer.try_recv() {
        received.push(delivery.bytes.to_vec());
    }
    received
}

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

/// Each viewer gets the same output; neither owns the terminal (grill Q6).
#[test]
fn every_attached_viewer_receives_the_same_output() {
    let mut stream = TerminalStream::new();
    let mut first = stream.attach();
    let mut second = stream.attach();

    let sequence = stream.advance();
    stream.deliver(sequence, b"shared output");

    assert_eq!(drain(&mut first), vec![b"shared output".to_vec()]);
    assert_eq!(
        drain(&mut second),
        vec![b"shared output".to_vec()],
        "one viewer's read consumed another's copy"
    );
}

#[test]
fn a_delivery_carries_the_epoch_and_sequence_it_belongs_to() {
    let mut stream = TerminalStream::new();
    let mut viewer = stream.attach();

    let sequence = stream.advance();
    stream.deliver(sequence, b"output");

    let delivery = viewer.try_recv().expect("a delivery");
    assert_eq!(delivery.epoch, Epoch(0));
    assert_eq!(delivery.sequence, Sequence(0));
}

/// Bytes from a screen shape that no longer exists must not be replayed into a
/// reflowed replica, so a reflow drops every viewer and each is owed a fresh
/// snapshot.
#[test]
fn a_reflow_ends_every_viewers_stream() {
    let mut stream = TerminalStream::new();
    let mut viewer = stream.attach();
    assert_eq!(stream.viewers(), 1);

    stream.open_epoch();

    assert_eq!(stream.viewers(), 0);
    let sequence = stream.advance();
    stream.deliver(sequence, b"after the reflow");
    assert!(
        drain(&mut viewer).is_empty(),
        "a viewer was fed bytes from a screen shape it had left"
    );
}

/// The budget measures what is still waiting, not what has ever been sent. A
/// viewer that keeps up must keep receiving for as long as it likes — an
/// earlier version counted cumulative bytes and dropped a healthy client the
/// moment a session had produced four megabytes in total.
#[test]
fn a_viewer_that_keeps_up_is_never_dropped_however_much_flows() {
    let mut stream = TerminalStream::new();
    let mut viewer = stream.attach();
    let chunk = vec![b'x'; 64 * 1024];

    // Four times the whole budget, drained as it arrives.
    let rounds = (SUBSCRIBER_QUEUE_BYTES / chunk.len()) * 4;
    for _ in 0..rounds {
        let sequence = stream.advance();
        stream.deliver(sequence, &chunk);
        assert!(!drain(&mut viewer).is_empty(), "a delivery went missing");
    }

    assert_eq!(
        stream.viewers(),
        1,
        "a viewer that read everything was dropped anyway"
    );
}

/// A viewer missing bytes out of the middle would render a screen that looks
/// plausible and is wrong. Losing the whole stream and resyncing is the
/// visible, honest failure.
#[test]
fn a_viewer_that_stops_reading_loses_its_stream_rather_than_its_middle() {
    let mut stream = TerminalStream::new();
    let mut viewer = stream.attach();
    let chunk = vec![b'x'; 64 * 1024];

    // Never drained, so the budget fills and stays full.
    for _ in 0..(SUBSCRIBER_QUEUE_BYTES / chunk.len() + 2) {
        let sequence = stream.advance();
        stream.deliver(sequence, &chunk);
    }

    assert_eq!(
        stream.viewers(),
        0,
        "a viewer past its budget kept receiving"
    );
    // What it did receive is a prefix, never a stream with a hole in it.
    let received = drain(&mut viewer);
    assert!(!received.is_empty());
    assert!(received.iter().all(|bytes| bytes.len() == chunk.len()));
}

/// One stalled viewer must not shrink what another may buffer, and must not
/// slow the stream itself.
#[test]
fn one_stalled_viewer_leaves_the_others_untouched() {
    let mut stream = TerminalStream::new();
    let _stalled = stream.attach();
    let mut healthy = stream.attach();
    let chunk = vec![b'x'; 64 * 1024];

    for _ in 0..(SUBSCRIBER_QUEUE_BYTES / chunk.len() + 2) {
        let sequence = stream.advance();
        stream.deliver(sequence, &chunk);
        let _ = drain(&mut healthy);
    }

    assert_eq!(
        stream.viewers(),
        1,
        "the healthy viewer was dropped with the stalled one"
    );
    let sequence = stream.advance();
    stream.deliver(sequence, b"still flowing");
    assert_eq!(drain(&mut healthy), vec![b"still flowing".to_vec()]);
}

/// A viewer whose client detached is dropped rather than accumulating output
/// nobody will ever read.
#[test]
fn a_viewer_whose_client_left_is_dropped() {
    let mut stream = TerminalStream::new();
    let viewer = stream.attach();
    drop(viewer);

    let sequence = stream.advance();
    stream.deliver(sequence, b"output");

    assert_eq!(stream.viewers(), 0);
}

/// Delivering to nobody is not an error: a session runs whether or not anyone
/// is watching, which is the point of the daemon owning the screen.
#[test]
fn a_stream_with_no_viewers_still_advances() {
    let mut stream = TerminalStream::new();

    let sequence = stream.advance();
    stream.deliver(sequence, b"nobody is attached");

    assert_eq!(stream.next_sequence(), Sequence(1));
}

/// The per-viewer limit that actually refuses is the byte budget, not the
/// frame count. A viewer holding more deliveries than the old 256-frame bound
/// but well under four megabytes is keeping up by the policy Corral states.
#[test]
fn a_viewer_holding_more_frames_than_bytes_is_not_desynchronised() {
    let mut stream = TerminalStream::new();
    let viewer = stream.attach();
    let chunk = vec![0_u8; 8192];

    // Past the frame bound the byte budget used to be dominated by; 2.4 MiB,
    // still under SUBSCRIBER_QUEUE_BYTES.
    for _ in 0..300 {
        let sequence = stream.advance();
        stream.deliver(sequence, &chunk);
    }

    assert_eq!(
        stream.viewers(),
        1,
        "a viewer was dropped by a frame count while its byte budget was two thirds free"
    );
    drop(viewer);
}

/// And the byte budget does still refuse: it is the limit, not decoration.
#[test]
fn a_viewer_past_its_byte_budget_is_desynchronised() {
    let mut stream = TerminalStream::new();
    let viewer = stream.attach();
    let chunk = vec![0_u8; 8192];

    for _ in 0..(SUBSCRIBER_QUEUE_BYTES / 8192 + 2) {
        let sequence = stream.advance();
        stream.deliver(sequence, &chunk);
    }

    assert_eq!(stream.viewers(), 0, "the byte budget never refused");
    drop(viewer);
}

/// Nothing is copied for a session nobody is watching — which is most of them.
#[test]
fn delivering_with_no_viewers_is_a_no_op() {
    let mut stream = TerminalStream::new();

    let sequence = stream.advance();
    stream.deliver(sequence, b"nobody is here");

    assert_eq!(stream.viewers(), 0);
}
