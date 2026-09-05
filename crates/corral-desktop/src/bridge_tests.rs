use super::*;

use futures::StreamExt;
use tokio::io::AsyncWriteExt;

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

/// The window's room is the bound. With nobody reading, the socket stops
/// draining — from there corrald's own viewer budget supersedes the backlog —
/// and once the window reads again nothing on the way in has been lost.
#[tokio::test]
async fn with_nobody_reading_the_socket_stops_draining_and_nothing_is_lost() {
    let (daemon, desktop) = tokio::net::UnixStream::pair().expect("a socket pair");
    let (sender, mut inbound) = foreground::channel(INBOUND_FRAMES);
    let (from_daemon, _to_daemon) = desktop.into_split();
    let reader = tokio::spawn(read_channel(from_daemon, Vec::new(), sender));

    // Far more than the window's room and a socket's buffers together hold,
    // at the size a PTY read delivers.
    let total = INBOUND_FRAMES * 4;
    let mut writer = tokio::spawn(async move {
        let (_from_desktop, mut to_desktop) = daemon.into_split();
        for sequence in 0..total {
            let frame = TerminalFrame {
                kind: FrameKind::Delta,
                epoch: Epoch(0),
                sequence: Sequence(sequence as u64),
                payload: vec![b'x'; 1024],
            };
            let wire = frame.encode().expect("within the ceiling");
            to_desktop.write_all(&wire).await.expect("written");
        }
    });

    assert!(
        tokio::time::timeout(Duration::from_millis(500), &mut writer)
            .await
            .is_err(),
        "the socket kept draining with nobody reading"
    );

    for expected in 0..total {
        let frame = inbound.next().await.expect("a frame");
        assert_eq!(frame.sequence, Sequence(expected as u64));
    }
    writer.await.expect("the writer finished");
    reader.await.expect("the reader ended at end of stream");
}
