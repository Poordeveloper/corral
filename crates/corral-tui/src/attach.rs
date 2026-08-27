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
//! (`docs/decisions/2026-08-24-pr3-plan-grill.md` Q4). This crate's other
//! citations are PR4's grill; this file predates it and its own are PR3's, so
//! they name their document.
//!
//! The list opens sessions through here rather than composing terminals of its
//! own: Open is a full-screen takeover of the same attachment this module
//! already implements (`docs/decisions/2026-08-25-pr4-tui-grill.md` Q1).

use std::io::{Read, Write};
use std::os::fd::{AsFd, BorrowedFd};

use corral_client::{Connection, RequestError, TerminalChannel};
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

/// What one burst of typing means to an attached session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocalInput {
    /// Bytes to send to the daemon.
    Send(Vec<u8>),
    /// The detach byte arrived: send what came before it, then stop. What came
    /// after it was typed for whatever the person is going back to, so it is
    /// carried rather than dropped.
    Detach { before: Vec<u8>, after: Vec<u8> },
}

/// Everything the person types, read once for the whole surface.
///
/// One reader per process, deliberately. A thread parked in `read` cannot be
/// cancelled, so a second one started for an attach would sit in the same
/// queue and take keystrokes the first was waiting for — and the character
/// that vanished would be one a person meant for their agent. So the list and
/// the attachment it hands over to share this rather than each starting one.
pub struct LocalKeys {
    typed: tokio::sync::mpsc::Receiver<Vec<u8>>,
    /// Bytes read by one surface and meant for the next.
    ///
    /// A burst can carry the key that opens a session and the first thing the
    /// person meant to type into it, and the detach byte with what they typed
    /// after it. Whoever stops reading hands the rest back rather than
    /// dropping it — dropping it is the vanished character this type exists to
    /// prevent.
    unconsumed: Vec<u8>,
}

/// Whether this process has already claimed the local terminal's input.
static READING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

impl LocalKeys {
    /// Start reading the local terminal, or refuse because something already
    /// is.
    ///
    /// The claim is the process's and lasts as long as the process does. A
    /// thread parked in `read` cannot be woken, so the reader started here
    /// cannot be reclaimed: releasing the claim would let a second one start
    /// while the first is still able to take one keystroke off the terminal on
    /// its way out. `None` therefore means this process already started its
    /// reader, not that a `LocalKeys` is alive somewhere.
    ///
    /// It is reported rather than asserted because the consequence of getting
    /// it wrong is a keystroke disappearing rather than a crash.
    pub fn start() -> Option<Self> {
        if READING.swap(true, std::sync::atomic::Ordering::AcqRel) {
            return None;
        }

        // Bounded: a person cannot type faster than this drains, and an
        // unbounded queue in front of a socket only moves where memory grows.
        let (typed, received) = tokio::sync::mpsc::channel(64);
        std::thread::spawn(move || {
            let mut stdin = std::io::stdin();
            let mut buffer = [0_u8; 4096];
            while let Some(bytes) = read_local(&mut stdin, &mut buffer) {
                if typed.blocking_send(bytes).is_err() {
                    return;
                }
            }
        });

        Some(Self {
            typed: received,
            unconsumed: Vec::new(),
        })
    }

    /// The next bytes the person typed, or `None` once their terminal closed.
    pub async fn next(&mut self) -> Option<Vec<u8>> {
        if !self.unconsumed.is_empty() {
            return Some(std::mem::take(&mut self.unconsumed));
        }
        self.typed.recv().await
    }

    /// Hand back bytes read but not consumed, for whoever reads next.
    pub fn put_back(&mut self, bytes: Vec<u8>) {
        if bytes.is_empty() {
            return;
        }
        // In front of anything already waiting: these were typed first.
        let mut whole = bytes;
        whole.append(&mut self.unconsumed);
        self.unconsumed = whole;
    }
}

/// Why an Open did not happen, or how it ended.
#[derive(Debug)]
pub enum OpenFailed {
    /// The daemon would not grant this session's terminal, or could not be
    /// asked for it. A fact about the connection the grant was asked on.
    Refused(RequestError),
    /// The grant was made and its channel could not be opened. A second
    /// connection to the same rendezvous, so this says nothing about the one
    /// that granted it, and a caller must not treat it as if it did.
    Unopened(RequestError),
    /// The channel itself failed while the person was attached.
    Channel(std::io::Error),
}

/// Split a read from the local terminal at the detach byte.
///
/// Bytes before it are still the person's input and are delivered; the detach
/// byte itself never is. Pasted input counts: a 0x1C arriving in a burst
/// detaches exactly as a typed one does, because the client cannot tell them
/// apart and guessing would make detaching unreliable.
pub fn split_at_detach(bytes: &[u8]) -> LocalInput {
    match bytes.iter().position(|byte| *byte == DETACH_BYTE) {
        Some(at) => LocalInput::Detach {
            before: bytes[..at].to_vec(),
            after: bytes[at + 1..].to_vec(),
        },
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
            // Clear the visible screen only. `ESC[3J` would also erase saved
            // lines — the person's own shell history, from before they ever
            // attached — and every resize and resync replays a snapshot, so
            // resizing a window would wipe it again and again. Corral does not
            // own that buffer.
            out.write_all(b"\x1b[H\x1b[2J")?;
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
        FrameKind::Input | FrameKind::Resize | FrameKind::ResyncRequest => Ok(()),
        // The skippability rule has one owner, in the protocol crate, so a
        // receiver added later cannot quietly decide it differently.
        other if other.is_skippable() => Ok(()),
        other => {
            debug_assert!(false, "no rule for {other:?}");
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
/// viewers reassert forever (`docs/decisions/2026-08-24-pr3-plan-grill.md` Q6).
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

/// Apply every complete frame in the buffer, tracking the epoch snapshots name.
fn drain_frames(
    pending: &mut Vec<u8>,
    out: &mut impl Write,
    epoch: &mut Epoch,
) -> std::io::Result<()> {
    loop {
        match TerminalFrame::decode_from_daemon(pending) {
            // Not yet a whole frame: the rest is still on its way.
            Ok(None) => return Ok(()),
            Ok(Some((frame, consumed))) => {
                pending.drain(..consumed);
                if frame.kind == FrameKind::Snapshot {
                    *epoch = frame.epoch;
                }
                apply(&frame, out)?;
            }
            // A fault is never "wait for more". The offending header would
            // never be consumed, so the screen would freeze with no message
            // and no exit — indistinguishable from a hung agent.
            Err(error) => return Err(std::io::Error::other(error.to_string())),
        }
    }
}

/// Read whatever the local terminal has, or `None` once it has closed.
///
/// Undecoded on purpose: what a burst of bytes means depends on who is
/// listening — the attached session splits it at the detach byte, the list
/// reads it as keys — and the one reader must not decide that for both.
fn read_local(input: &mut impl Read, buffer: &mut [u8]) -> Option<Vec<u8>> {
    loop {
        return match input.read(buffer) {
            Ok(0) => None,
            Ok(read) => Some(buffer[..read].to_vec()),
            // A signal is not a person closing their terminal. Ending the
            // attach here would detach someone mid-session and report success.
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => None,
        };
    }
}

impl Geometry {
    pub fn of(stdin: &std::io::Stdin) -> Option<Self> {
        local_geometry(stdin.as_fd())
    }
}

/// Attach to a session and run its terminal until the person detaches.
///
/// The one path every surface takes: `corral attach`, `corral new` and the
/// list's Open all reach a session through here, so none of them can grow its
/// own idea of what attaching means.
///
/// The terminal must already be in raw mode. Whoever started `keys` owns that:
/// the reader must never be parked on a terminal still in line discipline, so
/// the two are established together and this is downstream of both.
pub async fn open(
    connection: &mut Connection,
    session_id: &str,
    keys: &mut LocalKeys,
) -> Result<(), OpenFailed> {
    // Bounded for the same reason every wait in this crate is: raw mode is on
    // by now and the person is looking at a screen nobody is drawing, so a
    // daemon that never answers must not be waited on forever (`crate::ANSWER`).
    let grant =
        match tokio::time::timeout(crate::ANSWER, connection.terminal_attach(session_id)).await {
            Ok(grant) => grant.map_err(OpenFailed::Refused)?,
            Err(_) => return Err(OpenFailed::Refused(no_answer())),
        };

    let endpoint = connection.endpoint().to_path_buf();
    let opened = tokio::time::timeout(
        crate::ANSWER,
        Connection::open_terminal_channel(&endpoint, &grant.attach_token),
    )
    .await;
    let channel = match opened {
        Ok(channel) => channel.map_err(OpenFailed::Unopened)?,
        Err(_) => return Err(OpenFailed::Unopened(no_answer())),
    };

    // The session's size is what the daemon reports; this terminal's is what
    // the person has. Told to `run` so it can reconcile them at once —
    // otherwise a 50x200 terminal renders a 24x80 session in the corner for
    // the whole attach, and an 80-column terminal wraps a 200-column session
    // into garbage.
    let session_geometry = Geometry {
        rows: grant.rows,
        cols: grant.cols,
    };

    run(channel, session_geometry, keys)
        .await
        .map_err(OpenFailed::Channel)
}

/// Run an attached terminal session until the person detaches or it ends.
///
/// Local input and daemon output are independent, so they are awaited
/// separately: a person typing must not wait for the screen, and the screen
/// must not wait for a keystroke. The keys arrive from the process's one
/// reader rather than a thread started here — see `LocalKeys`.
///
/// Raw mode is the caller's, for the same reason the reader is: one terminal
/// mode with one owner. A guard taken here would capture an already-raw
/// terminal as the state to restore, which is a restore that restores
/// nothing.
pub async fn run(
    channel: TerminalChannel,
    session_geometry: Geometry,
    keys: &mut LocalKeys,
) -> std::io::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let (mut from_daemon, mut to_daemon) = channel.stream.into_split();

    let mut stdout = std::io::stdout();
    // Bytes that arrived with the handshake are already terminal frames.
    let mut pending = channel.leftover;
    let mut buffer = [0_u8; 65536];
    // Seeded from the session, not from this terminal: the two are compared
    // below, and starting them equal would mean never reconciling them.
    let mut local = Some(session_geometry);
    // Adopted from what arrives rather than assumed: a resize opens a new
    // epoch, and input still labelled with the old one names a screen shape
    // that no longer exists.
    let mut epoch = Epoch(0);

    // Whatever came in with the handshake is already a frame.
    drain_frames(&mut pending, &mut stdout, &mut epoch)?;

    // A person who resizes their window and then just watches must still get a
    // correct screen, so the local size is checked on a tick rather than only
    // when a key is pressed. Polling instead of SIGWINCH keeps the client
    // single-threaded about its own geometry. The first tick fires
    // immediately, which is what reconciles this terminal with the session's.
    let mut geometry_check = tokio::time::interval(std::time::Duration::from_millis(250));
    geometry_check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            read = from_daemon.read(&mut buffer) => match read {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    pending.extend_from_slice(&buffer[..read]);
                    drain_frames(&mut pending, &mut stdout, &mut epoch)?;
                }
            },
            _ = geometry_check.tick() => {
                // A resize is sent only when this terminal's own size changed —
                // never because the daemon reported one, which would make two
                // differently sized viewers reassert forever
                // (`docs/decisions/2026-08-24-pr3-plan-grill.md` Q6). On a
                // tick rather than only on a keystroke: a person who resizes
                // and then just watches still needs a correct screen.
                let now = Geometry::of(&std::io::stdin());
                if now != local {
                    if let Some(geometry) = now
                        && let Ok(frame) = resize_frame(epoch, geometry).encode()
                        && to_daemon.write_all(&frame).await.is_err()
                    {
                        break;
                    }
                    local = now;
                }
            }
            typed = keys.next() => match typed {
                None => break,
                Some(bytes) => {
                    // The detach is decided here rather than by the reader,
                    // and in the order the bytes arrived: what came before it
                    // is still the person's input and is delivered first.
                    let (payload, detaching) = match split_at_detach(&bytes) {
                        LocalInput::Send(bytes) => (bytes, false),
                        LocalInput::Detach { before, after } => {
                            keys.put_back(after);
                            (before, true)
                        }
                    };

                    if !payload.is_empty() {
                        match input_frame(epoch, payload).encode() {
                            Ok(frame) => {
                                if to_daemon.write_all(&frame).await.is_err() {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }

                    if detaching {
                        break;
                    }
                }
            },
        }
    }

    // Closing the channel is not ending the session: the process keeps running,
    // which is what detaching means.
    let _ = to_daemon.shutdown().await;
    // Leave the person's terminal on a fresh line rather than wherever the
    // session's cursor happened to be. Written rather than printed, because
    // the terminal is still raw — the caller holds it — and a bare newline
    // there moves down without returning to the first column.
    stdout.write_all(b"\r\n")?;
    stdout.flush()?;
    Ok(())
}

/// What ran out of patience, said the way a request failure is said.
fn no_answer() -> RequestError {
    RequestError::Protocol {
        detail: format!("nothing within {} seconds", crate::ANSWER.as_secs()),
    }
}

impl std::fmt::Display for OpenFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(error) => write!(f, "{error}"),
            Self::Unopened(error) => write!(f, "the terminal channel did not open: {error}"),
            Self::Channel(error) => write!(f, "the terminal channel ended: {error}"),
        }
    }
}

impl std::error::Error for OpenFailed {}

#[cfg(test)]
#[path = "attach_tests.rs"]
mod tests;
