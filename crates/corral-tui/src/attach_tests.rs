use super::*;

/// The detach byte never reaches the daemon, and what came before it still
/// does: a person who typed `abc` then detached meant to send `abc`. What they
/// typed after it was meant for the list they are going back to, and is
/// carried there rather than dropped.
#[test]
fn the_detach_byte_ends_input_and_is_never_forwarded() {
    let typed = [b'a', b'b', b'c', DETACH_BYTE, b'd'];

    let outcome = split_at_detach(&typed);

    assert_eq!(
        outcome,
        LocalInput::Detach {
            before: vec![b'a', b'b', b'c'],
            after: vec![b'd'],
        }
    );
    match outcome {
        LocalInput::Detach { before, after } => {
            assert!(
                !before.contains(&DETACH_BYTE) && !after.contains(&DETACH_BYTE),
                "the detach byte was about to be passed on"
            );
        }
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
        LocalInput::Detach {
            before: b"some pasted text".to_vec(),
            after: b"with more after".to_vec(),
        },
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
    assert_eq!(
        split_at_detach(&[DETACH_BYTE]),
        LocalInput::Detach {
            before: vec![],
            after: vec![],
        }
    );
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
        kind: FrameKind::from_byte(99),
        epoch: Epoch(0),
        sequence: Sequence(0),
        payload: b"a kind from later".to_vec(),
    };
    let mut out = Vec::new();

    apply(&frame, &mut out).expect("applied");

    assert!(out.is_empty());
}

/// ADR 0017's `Geometry` (7) and `Palette` (8), applied by the build that
/// predates them: nothing of either reaches the person's terminal, and the
/// snapshot that follows still does. The pre-ADR acceptance check
/// (docs/decisions/2026-09-05-adr-0017-grill.md Q5).
#[test]
fn geometry_and_palette_frames_write_nothing_and_the_snapshot_after_them_still_applies() {
    let mut out = Vec::new();
    for (kind, payload) in [
        (7, b"\x00\x1e\x00\x64".to_vec()),
        (8, b"\x1b]4;1;rgb:12/34/56\x07".to_vec()),
    ] {
        let frame = TerminalFrame {
            kind: FrameKind::from_byte(kind),
            epoch: Epoch(3),
            sequence: Sequence(9),
            payload,
        };
        apply(&frame, &mut out).expect("applied");
        assert!(out.is_empty(), "kind {kind} reached the terminal");
    }
    let snapshot = TerminalFrame {
        kind: FrameKind::Snapshot,
        epoch: Epoch(3),
        sequence: Sequence(9),
        payload: b"the screen".to_vec(),
    };
    apply(&snapshot, &mut out).expect("applied");
    assert!(out.ends_with(b"the screen"));
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

/// A snapshot clears the visible screen and nothing else. `ESC[3J` erases
/// saved lines — the person's own shell history from before they attached —
/// and a snapshot is replayed on every attach, resize and resync.
#[test]
fn applying_a_snapshot_does_not_erase_the_persons_scrollback() {
    let mut out = Vec::new();
    let frame = TerminalFrame {
        kind: FrameKind::Snapshot,
        epoch: Epoch(0),
        sequence: Sequence(0),
        payload: b"a screen".to_vec(),
    };

    apply(&frame, &mut out).expect("applied");

    let written = String::from_utf8_lossy(&out);
    assert!(
        !written.contains("[3J"),
        "the client erased saved lines it does not own: {written:?}"
    );
    assert!(
        written.contains("[2J"),
        "the visible screen was not cleared"
    );
}
