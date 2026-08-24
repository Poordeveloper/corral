use super::*;

/// The detach byte never reaches the daemon, and what came before it still
/// does: a person who typed `abc` then detached meant to send `abc`.
#[test]
fn the_detach_byte_ends_input_and_is_never_forwarded() {
    let typed = [b'a', b'b', b'c', DETACH_BYTE, b'd'];

    let outcome = split_at_detach(&typed);

    assert_eq!(outcome, LocalInput::Detach(vec![b'a', b'b', b'c']));
    match outcome {
        LocalInput::Detach(bytes) => assert!(
            !bytes.contains(&DETACH_BYTE),
            "the detach byte was about to be sent to the child"
        ),
        other => panic!("{other:?}"),
    }
}

/// Pasted input counts. The client cannot tell a typed 0x1C from a pasted one,
/// and guessing would make detaching unreliable — so it detaches either way,
/// which is the limitation recorded in the ADR rather than a surprise.
#[test]
fn a_pasted_detach_byte_detaches_exactly_as_a_typed_one_does() {
    let pasted = b"some pasted text\x1cwith more after";

    let outcome = split_at_detach(pasted);

    assert_eq!(
        outcome,
        LocalInput::Detach(b"some pasted text".to_vec()),
        "a detach byte arriving in a burst was forwarded to the child"
    );
}

#[test]
fn ordinary_input_passes_through_untouched() {
    let typed = b"\x1b[Aecho hello\r";

    assert_eq!(split_at_detach(typed), LocalInput::Send(typed.to_vec()));
}

#[test]
fn a_detach_byte_alone_sends_nothing_and_detaches() {
    assert_eq!(split_at_detach(&[DETACH_BYTE]), LocalInput::Detach(vec![]));
}

/// A snapshot is the whole screen, so it clears first: replaying it over stale
/// text would leave rows nothing overwrote.
#[test]
fn a_snapshot_clears_before_it_replays() {
    let frame = TerminalFrame {
        kind: FrameKind::Snapshot,
        epoch: Epoch(3),
        sequence: Sequence(9),
        payload: b"the screen".to_vec(),
    };
    let mut out = Vec::new();

    apply(&frame, &mut out).expect("applied");

    assert!(out.starts_with(b"\x1b["), "no clear preceded the snapshot");
    assert!(out.ends_with(b"the screen"));
}

/// Deltas are raw PTY output and are replayed unmodified — no translation, no
/// escaping, nothing the daemon did not send.
#[test]
fn a_delta_is_replayed_byte_for_byte() {
    let payload: Vec<u8> = (0..=255_u8).collect();
    let frame = TerminalFrame {
        kind: FrameKind::Delta,
        epoch: Epoch(0),
        sequence: Sequence(1),
        payload: payload.clone(),
    };
    let mut out = Vec::new();

    apply(&frame, &mut out).expect("applied");

    assert_eq!(out, payload);
}

/// A frame kind this build does not know is skipped, not rendered: writing an
/// unknown payload to a terminal would put a future protocol's bytes on a
/// person's screen.
#[test]
fn an_unknown_frame_writes_nothing() {
    let frame = TerminalFrame {
        kind: FrameKind::Unknown(99),
        epoch: Epoch(0),
        sequence: Sequence(0),
        payload: b"a kind from later".to_vec(),
    };
    let mut out = Vec::new();

    apply(&frame, &mut out).expect("applied");

    assert!(out.is_empty());
}

#[test]
fn a_resize_frame_carries_this_clients_own_geometry() {
    let geometry = Geometry {
        rows: 44,
        cols: 155,
    };

    let frame = resize_frame(Epoch(2), geometry);

    assert_eq!(frame.kind, FrameKind::Resize);
    assert_eq!(frame.epoch, Epoch(2));
    assert_eq!(
        frame.payload,
        vec![0, 44, 0, 155],
        "the geometry was not encoded as rows then columns"
    );
}

#[test]
fn an_input_frame_carries_exactly_what_was_typed() {
    let frame = input_frame(Epoch(1), b"\x03".to_vec());

    assert_eq!(frame.kind, FrameKind::Input);
    assert_eq!(frame.payload, b"\x03".to_vec());
}
