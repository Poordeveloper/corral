use super::*;

use futures::StreamExt;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use tokio::task::JoinHandle;

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

const MIB: usize = 1024 * 1024;

/// A reader on the Desktop's end of a socket pair, with the room the bridge
/// gives one: the daemon's end to write into, the window's end to read from.
fn attached() -> (UnixStream, foreground::Receiver<Delivery>, JoinHandle<()>) {
    let (daemon, desktop) = UnixStream::pair().expect("a socket pair");
    let (sender, inbound) = foreground::channel(INBOUND_QUEUE_FRAMES);
    let (from_daemon, _to_daemon) = desktop.into_split();
    let room = Arc::new(Semaphore::new(INBOUND_QUEUE_BYTES as usize));
    let reader = tokio::spawn(read_channel(from_daemon, Vec::new(), sender, room));
    (daemon, inbound, reader)
}

fn frame(kind: FrameKind, sequence: usize, bytes: usize) -> TerminalFrame {
    TerminalFrame {
        kind,
        epoch: Epoch(0),
        sequence: Sequence(sequence as u64),
        payload: vec![b'x'; bytes],
    }
}

/// The daemon writing frames, as fast as the socket takes them.
fn pump(daemon: UnixStream, frames: Vec<TerminalFrame>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let (_from_desktop, mut to_desktop) = daemon.into_split();
        for frame in frames {
            let wire = frame.encode().expect("within the ceiling");
            to_desktop.write_all(&wire).await.expect("written");
        }
    })
}

/// Whether the daemon is still writing after a wait no socket needs.
async fn still_writing(writer: &mut JoinHandle<()>) -> bool {
    tokio::time::timeout(Duration::from_millis(500), writer)
        .await
        .is_err()
}

/// The next `count` frames, in sequence order, each dropped as it is read.
async fn take(inbound: &mut foreground::Receiver<Delivery>, from: usize, count: usize) {
    for expected in from..from + count {
        let delivery = inbound.next().await.expect("a frame");
        assert_eq!(delivery.frame.sequence, Sequence(expected as u64));
    }
}

/// The window's room is the bound. With nobody reading, the socket stops
/// draining — from there corrald's own viewer budget supersedes the backlog —
/// and once the window reads again nothing on the way in has been lost.
#[tokio::test]
async fn with_nobody_reading_the_socket_stops_draining_and_nothing_is_lost() {
    let (daemon, mut inbound, reader) = attached();

    // Far more than the room and a socket's buffers together hold, at the
    // size a PTY read delivers.
    let total = INBOUND_QUEUE_BYTES as usize / 1024 * 4;
    let frames = (0..total)
        .map(|sequence| frame(FrameKind::Delta, sequence, 1024))
        .collect();
    let mut writer = pump(daemon, frames);

    assert!(
        still_writing(&mut writer).await,
        "the socket kept draining with nobody reading"
    );

    take(&mut inbound, 0, total).await;
    writer.await.expect("the writer finished");
    reader.await.expect("the reader ended at end of stream");
}

/// The room is bytes, not frames: a few snapshots the size the daemon aims
/// for (`SNAPSHOT_TARGET_BYTES`) fill it as surely as thousands of deltas.
#[tokio::test]
async fn a_few_large_frames_fill_the_room() {
    let (daemon, mut inbound, reader) = attached();

    let total = 4 * INBOUND_QUEUE_BYTES as usize / MIB;
    let frames = (0..total)
        .map(|sequence| frame(FrameKind::Snapshot, sequence, MIB))
        .collect();
    let mut writer = pump(daemon, frames);

    assert!(
        still_writing(&mut writer).await,
        "the socket kept draining past the byte budget"
    );

    take(&mut inbound, 0, total).await;
    writer.await.expect("the writer finished");
    reader.await.expect("the reader ended at end of stream");
}

/// A frame larger than the whole room — a snapshot may be 16 MiB — is not
/// refused, which would leave the channel stuck forever; it waits for the
/// room to empty and is admitted alone. The reader assembles it off the
/// socket first, so the socket drains; what it may not do is deliver it
/// beside another frame.
#[tokio::test]
async fn a_frame_past_the_whole_room_waits_for_it_to_empty_and_is_admitted_alone() {
    let (daemon, mut inbound, reader) = attached();

    let large = 2 * INBOUND_QUEUE_BYTES as usize;
    let writer = pump(
        daemon,
        vec![
            frame(FrameKind::Delta, 0, 1024),
            frame(FrameKind::Snapshot, 1, large),
        ],
    );
    writer.await.expect("the writer finished");

    // The small frame is held, so its room is not back; a wait no decode
    // needs, so the reader has certainly asked for the large frame's.
    let held = inbound.next().await.expect("the small frame");
    assert_eq!(held.frame.sequence, Sequence(0));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        inbound.try_recv().is_err(),
        "the large frame was delivered beside another"
    );

    drop(held);
    let delivery = inbound.next().await.expect("the large frame");
    assert_eq!(delivery.frame.sequence, Sequence(1));
    assert_eq!(delivery.frame.payload.len(), large);
    reader.await.expect("the reader ended at end of stream");
}
