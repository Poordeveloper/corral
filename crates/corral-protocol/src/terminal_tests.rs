use super::*;

fn frame(kind: FrameKind, payload: &[u8]) -> TerminalFrame {
    TerminalFrame {
        kind,
        epoch: Epoch(7),
        sequence: Sequence(42),
        payload: payload.to_vec(),
    }
}

#[test]
fn every_known_kind_round_trips_through_its_wire_byte() {
    for kind in [
        FrameKind::Snapshot,
        FrameKind::Delta,
        FrameKind::Input,
        FrameKind::Resize,
        FrameKind::ResyncRequest,
        FrameKind::ChannelError,
    ] {
        assert_eq!(FrameKind::from_byte(kind.as_byte()), kind);
    }
}

#[test]
fn a_frame_round_trips_with_its_epoch_sequence_and_payload() {
    let original = frame(FrameKind::Delta, b"\x1b[2Jhello\n\x00\xff");

    let encoded = original.encode().expect("encode");
    let (decoded, consumed) = TerminalFrame::decode_from_daemon(&encoded)
        .expect("decode")
        .expect("a complete frame");

    assert_eq!(decoded, original);
    assert_eq!(consumed, encoded.len());
}

/// PTY bytes are replayed unmodified — no newline translation, no escaping.
/// A framing that could not carry arbitrary bytes would force exactly that.
#[test]
fn a_payload_of_arbitrary_bytes_survives_unchanged() {
    let payload: Vec<u8> = (0..=255_u8).collect();
    let original = frame(FrameKind::Delta, &payload);

    let encoded = original.encode().expect("encode");
    let (decoded, _) = TerminalFrame::decode_from_daemon(&encoded)
        .expect("decode")
        .expect("a complete frame");

    assert_eq!(decoded.payload, payload);
}

/// A peer that learns a new frame kind must not become undecodable to an older
/// one. The length prefix says exactly how much to skip, so an unknown kind is
/// survivable on a stream that cannot be resynchronised by scanning.
#[test]
fn an_unknown_frame_kind_decodes_and_is_skippable() {
    let mut encoded = frame(FrameKind::Delta, b"payload")
        .encode()
        .expect("encode");
    encoded[0] = 200;

    let (decoded, consumed) = TerminalFrame::decode_from_daemon(&encoded)
        .expect("decode")
        .expect("a complete frame");

    assert!(matches!(decoded.kind, FrameKind::Unknown(_)));
    assert_eq!(decoded.kind.as_byte(), 200);
    assert!(decoded.kind.is_skippable());
    assert_eq!(
        consumed,
        encoded.len(),
        "an unknown kind must consume exactly its declared length"
    );
}

/// An unknown kind in the middle of a stream leaves the frames after it
/// readable — the property that makes the channel additively evolvable.
#[test]
fn a_stream_survives_an_unknown_kind_between_known_ones() {
    let mut stream = frame(FrameKind::Delta, b"before").encode().expect("encode");
    let mut future = frame(FrameKind::from_byte(77), b"a kind from later")
        .encode()
        .expect("encode");
    let mut after = frame(FrameKind::Delta, b"after").encode().expect("encode");
    stream.append(&mut future);
    stream.append(&mut after);

    let mut offset = 0;
    let mut payloads = Vec::new();
    while let Some((decoded, consumed)) =
        TerminalFrame::decode_from_daemon(&stream[offset..]).expect("decode")
    {
        if !decoded.kind.is_skippable() {
            payloads.push(decoded.payload);
        }
        offset += consumed;
    }

    assert_eq!(payloads, vec![b"before".to_vec(), b"after".to_vec()]);
}

/// Every byte is either an assigned kind that round-trips, or an unknown one
/// that stays unknown. Nothing in between: a kind that re-encoded as a
/// different one would be a frame that lies about itself.
#[test]
fn every_byte_is_either_an_assigned_kind_or_stays_unknown() {
    for raw in 0..=u8::MAX {
        let kind = FrameKind::from_byte(raw);

        assert_eq!(kind.as_byte(), raw, "byte {raw} did not survive");
        assert_eq!(
            FrameKind::from_byte(kind.as_byte()),
            kind,
            "byte {raw} decoded to something else on the way back"
        );
        assert_eq!(
            kind.is_skippable(),
            matches!(kind, FrameKind::Unknown(_)),
            "byte {raw} disagrees about whether it may be skipped"
        );
    }
}

#[test]
fn a_partial_frame_is_not_yet_a_frame() {
    let encoded = frame(FrameKind::Snapshot, b"a screen")
        .encode()
        .expect("encode");

    for truncated in 0..encoded.len() {
        assert!(
            TerminalFrame::decode_from_daemon(&encoded[..truncated])
                .expect("a short buffer is not a fault")
                .is_none(),
            "{truncated} bytes decoded as a complete frame"
        );
    }
}

/// A peer cannot make the other allocate without bound. The limit is this
/// channel's own, derived from the snapshot ceiling rather than shared with
/// the much smaller RPC limit.
#[test]
fn a_frame_declaring_more_than_the_limit_is_refused_before_allocating() {
    let mut encoded = frame(FrameKind::Snapshot, b"small")
        .encode()
        .expect("encode");
    let declared = (MAX_TERMINAL_FRAME_BYTES + 1) as u32;
    encoded[17..21].copy_from_slice(&declared.to_be_bytes());

    let error = TerminalFrame::decode_from_daemon(&encoded).expect_err("refused");

    assert!(
        matches!(error, TerminalFrameError::Oversize { .. }),
        "{error}"
    );
}

/// The two channels answer different questions, so their limits are different
/// numbers. A snapshot is the largest legitimate message on the terminal
/// channel and would not fit the semantic channel's much smaller frame.
#[test]
fn the_terminal_limit_admits_a_frame_the_rpc_limit_would_refuse() {
    let snapshot_sized = vec![0_u8; crate::framing::MAX_FRAME_BYTES + 1];

    let frame = TerminalFrame {
        kind: FrameKind::Snapshot,
        epoch: Epoch(0),
        sequence: Sequence(0),
        payload: snapshot_sized,
    };

    assert!(
        frame.encode().is_ok(),
        "a snapshot larger than an RPC frame was refused by the terminal channel"
    );
}

/// The client direction is bounded by what a client may make the daemon hold,
/// and the bound is applied to the declared length — before the body is waited
/// for, and before any of it is copied. A header alone must not be able to
/// reserve sixteen megabytes per connection.
#[test]
fn a_client_frame_past_the_client_limit_is_refused_on_its_header_alone() {
    let mut header = frame(FrameKind::Resize, b"").encode().expect("encode");
    let declared = (MAX_CLIENT_FRAME_BYTES + 1) as u32;
    header[17..21].copy_from_slice(&declared.to_be_bytes());

    // The header alone, with none of the body it declares.
    let error = TerminalFrame::decode_from_client(&header).expect_err("refused");

    assert!(
        matches!(
            error,
            TerminalFrameError::Oversize {
                limit: MAX_CLIENT_FRAME_BYTES,
                ..
            }
        ),
        "{error}"
    );
}

/// The same declaration is legitimate from the daemon: a snapshot is the
/// largest message on this channel, and the two directions answer different
/// questions.
#[test]
fn the_same_length_is_admitted_from_the_daemon() {
    let mut header = frame(FrameKind::Snapshot, b"").encode().expect("encode");
    let declared = (MAX_CLIENT_FRAME_BYTES + 1) as u32;
    header[17..21].copy_from_slice(&declared.to_be_bytes());

    // `Ok(None)`: within the limit, body not yet arrived.
    assert_eq!(
        TerminalFrame::decode_from_daemon(&header).expect("admitted"),
        None
    );
}
