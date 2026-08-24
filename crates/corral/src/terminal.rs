//! The interactive attach client: a byte pipe between a person's terminal and
//! the one `corrald` owns.
//!
//! The client is deliberately dumb about content. It replays the daemon's
//! bytes into the local terminal and sends the local terminal's bytes back,
//! because the person's own terminal *is* the replica: it already knows its
//! mode bits, so nothing here has to model them (`ARCHITECTURE.md` §3).
//!
//! One byte is not passed through. `Ctrl-\` detaches, unconditionally, and is
//! never forwarded — so a literal 0x1C cannot reach the child through Corral's
//! M1 attach. That is a limitation we chose, recorded rather than discovered
//! (grill Q4).

use std::io::{Read, Write};
use std::os::fd::{AsFd, BorrowedFd};

use corral_protocol::terminal::{Epoch, FrameKind, Sequence, TerminalFrame};

/// The byte that detaches. `Ctrl-\`, ASCII FS.
pub const DETACH_BYTE: u8 = 0x1C;

/// The terminal's geometry, as this client sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub rows: u16,
    pub cols: u16,
}

/// Restores the terminal's line discipline however the attach ends.
///
/// A person left in raw mode after a crash has a shell that no longer echoes
/// or handles Ctrl-C, and no obvious way to notice why.
pub struct RawMode {
    stdin: std::io::Stdin,
    original: rustix::termios::Termios,
}

impl RawMode {
    pub fn enter() -> std::io::Result<Option<Self>> {
        let stdin = std::io::stdin();
        let Ok(original) = rustix::termios::tcgetattr(stdin.as_fd()) else {
            // Not a terminal: piped input is legal, it just cannot be raw.
            return Ok(None);
        };

        let mut raw = original.clone();
        raw.make_raw();
        rustix::termios::tcsetattr(stdin.as_fd(), rustix::termios::OptionalActions::Now, &raw)
            .map_err(std::io::Error::from)?;

        Ok(Some(Self { stdin, original }))
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = rustix::termios::tcsetattr(
            self.stdin.as_fd(),
            rustix::termios::OptionalActions::Now,
            &self.original,
        );
    }
}

/// This terminal's current size, if it is a terminal at all.
pub fn local_geometry(fd: BorrowedFd<'_>) -> Option<Geometry> {
    let size = rustix::termios::tcgetwinsize(fd).ok()?;
    Some(Geometry {
        rows: size.ws_row,
        cols: size.ws_col,
    })
}

/// What reading local input produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocalInput {
    /// Bytes to send to the daemon.
    Send(Vec<u8>),
    /// The detach byte arrived: send what came before it, then stop.
    Detach(Vec<u8>),
    /// The local terminal closed.
    Closed,
}

/// Split a read from the local terminal at the detach byte.
///
/// Bytes before it are still the person's input and are delivered; the detach
/// byte itself never is. Pasted input counts: a 0x1C arriving in a burst
/// detaches exactly as a typed one does, because the client cannot tell them
/// apart and guessing would make detaching unreliable.
pub fn split_at_detach(bytes: &[u8]) -> LocalInput {
    match bytes.iter().position(|byte| *byte == DETACH_BYTE) {
        Some(at) => LocalInput::Detach(bytes[..at].to_vec()),
        None => LocalInput::Send(bytes.to_vec()),
    }
}

/// Frames the daemon sent, applied to the local terminal.
///
/// A snapshot replaces what is on screen; a delta appends to it. The client
/// keeps no model of either — it writes bytes and lets the person's own
/// terminal be the emulator.
pub fn apply(frame: &TerminalFrame, out: &mut impl Write) -> std::io::Result<()> {
    match frame.kind {
        FrameKind::Snapshot => {
            // Clear first: a snapshot is the whole screen, and replaying it
            // over stale text would leave rows nothing overwrote.
            out.write_all(b"\x1b[H\x1b[2J\x1b[3J")?;
            out.write_all(&frame.payload)?;
            out.flush()
        }
        FrameKind::Delta => {
            out.write_all(&frame.payload)?;
            out.flush()
        }
        FrameKind::ChannelError => {
            out.write_all(b"\r\n")?;
            out.write_all(&frame.payload)?;
            out.write_all(b"\r\n")?;
            out.flush()
        }
        // Kinds only a client sends, and kinds this build does not know. Both
        // are skipped: the length prefix already said how much to drop.
        FrameKind::Input | FrameKind::Resize | FrameKind::ResyncRequest | FrameKind::Unknown(_) => {
            Ok(())
        }
    }
}

/// An input frame carrying bytes the local terminal produced.
pub fn input_frame(epoch: Epoch, bytes: Vec<u8>) -> TerminalFrame {
    TerminalFrame {
        kind: FrameKind::Input,
        epoch,
        sequence: Sequence(0),
        payload: bytes,
    }
}

/// A resize frame carrying this client's own desired geometry.
///
/// Sent only when the local terminal actually changed size — never because the
/// daemon reported a new geometry, which would make two differently sized
/// viewers reassert forever (grill Q6).
pub fn resize_frame(epoch: Epoch, geometry: Geometry) -> TerminalFrame {
    let mut payload = Vec::with_capacity(4);
    payload.extend_from_slice(&geometry.rows.to_be_bytes());
    payload.extend_from_slice(&geometry.cols.to_be_bytes());
    TerminalFrame {
        kind: FrameKind::Resize,
        epoch,
        sequence: Sequence(0),
        payload,
    }
}

/// Read what the local terminal has, without blocking the caller forever.
pub fn read_local(input: &mut impl Read, buffer: &mut [u8]) -> LocalInput {
    match input.read(buffer) {
        Ok(0) | Err(_) => LocalInput::Closed,
        Ok(read) => split_at_detach(&buffer[..read]),
    }
}

impl Geometry {
    pub fn of(stdin: &std::io::Stdin) -> Option<Self> {
        local_geometry(stdin.as_fd())
    }
}

#[cfg(test)]
#[path = "terminal_tests.rs"]
mod tests;

/// Run an attached terminal session until the person detaches or it ends.
///
/// Local input and daemon output are independent, so they run separately: a
/// person typing must not wait for the screen, and the screen must not wait
/// for a keystroke. Reading the local terminal blocks, so it runs on its own
/// thread and hands bytes over.
pub async fn run(channel: tokio::net::UnixStream) -> std::io::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let raw = RawMode::enter()?;
    let (mut from_daemon, mut to_daemon) = channel.into_split();
    // The epoch the daemon's first snapshot names; every later snapshot
    // updates it.
    let epoch = Epoch(0);

    // Bounded: a person cannot type faster than this drains, and an unbounded
    // queue in front of a socket only moves where the memory grows.
    let (typed, mut keystrokes) = tokio::sync::mpsc::channel::<Vec<u8>>(64);
    let detached = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reader_detached = std::sync::Arc::clone(&detached);

    std::thread::spawn(move || {
        let mut stdin = std::io::stdin();
        let mut buffer = [0_u8; 4096];
        loop {
            match read_local(&mut stdin, &mut buffer) {
                LocalInput::Send(bytes) => {
                    if typed.blocking_send(bytes).is_err() {
                        return;
                    }
                }
                LocalInput::Detach(bytes) => {
                    if !bytes.is_empty() {
                        let _ = typed.blocking_send(bytes);
                    }
                    reader_detached.store(true, std::sync::atomic::Ordering::Release);
                    return;
                }
                LocalInput::Closed => return,
            }
        }
    });

    let mut stdout = std::io::stdout();
    let mut pending = Vec::new();
    let mut buffer = [0_u8; 65536];
    let mut local = Geometry::of(&std::io::stdin());
    // Adopted from what arrives rather than assumed: a resize opens a new
    // epoch, and input still labelled with the old one names a screen shape
    // that no longer exists.
    let mut epoch = epoch;

    loop {
        tokio::select! {
            read = from_daemon.read(&mut buffer) => match read {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    pending.extend_from_slice(&buffer[..read]);
                    while let Ok(Some((frame, consumed))) = TerminalFrame::decode(&pending) {
                        pending.drain(..consumed);
                        if frame.kind == FrameKind::Snapshot {
                            epoch = frame.epoch;
                        }
                        apply(&frame, &mut stdout)?;
                    }
                }
            },
            bytes = keystrokes.recv() => match bytes {
                None => break,
                Some(bytes) => {
                    // A resize is sent only when this terminal's own size
                    // changed — never because the daemon reported one, which
                    // would make two differently sized viewers reassert
                    // forever (grill Q6). Checking here rather than on SIGWINCH
                    // keeps the client single-threaded about its own geometry.
                    let now = Geometry::of(&std::io::stdin());
                    if now != local {
                        if let Some(geometry) = now
                            && let Ok(frame) = resize_frame(epoch, geometry).encode()
                        {
                            let _ = to_daemon.write_all(&frame).await;
                        }
                        local = now;
                    }

                    match input_frame(epoch, bytes).encode() {
                        Ok(frame) => {
                            if to_daemon.write_all(&frame).await.is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }

                    if detached.load(std::sync::atomic::Ordering::Acquire) {
                        break;
                    }
                }
            },
        }

        if detached.load(std::sync::atomic::Ordering::Acquire) && keystrokes.is_empty() {
            break;
        }
    }

    // Closing the channel is not ending the session: the process keeps running,
    // which is what detaching means.
    let _ = to_daemon.shutdown().await;
    drop(raw);
    // Leave the person's terminal on a fresh line rather than wherever the
    // session's cursor happened to be.
    println!();
    Ok(())
}
