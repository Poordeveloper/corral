use super::*;

/// The daemon ends a channel over a client frame past `MAX_CLIENT_FRAME_BYTES`,
/// so a paste that size crosses as several frames its decoder accepts, in
/// order, carrying every byte once.
#[test]
fn input_past_the_client_ceiling_crosses_as_frames_the_daemon_accepts() {
    let pasted: Vec<u8> = (0..=MAX_CLIENT_FRAME_BYTES)
        .map(|index| (index % 251) as u8)
        .collect();

    let frames = input_frames(Epoch(7), &pasted);

    assert_eq!(frames.len(), 2);
    let mut carried = Vec::new();
    for frame in &frames {
        assert_eq!(frame.kind, FrameKind::Input);
        assert_eq!(frame.epoch, Epoch(7));
        let wire = frame.encode().expect("a frame within the wire ceiling");
        let (decoded, consumed) = TerminalFrame::decode_from_client(&wire)
            .expect("the daemon's decoder accepts it")
            .expect("a whole frame");
        assert_eq!(consumed, wire.len());
        carried.extend_from_slice(&decoded.payload);
    }
    assert_eq!(carried, pasted);
}

#[test]
fn input_within_the_ceiling_is_one_frame_and_none_is_no_frame() {
    assert_eq!(input_frames(Epoch(0), b"\x03").len(), 1);
    assert!(input_frames(Epoch(0), b"").is_empty());
}
