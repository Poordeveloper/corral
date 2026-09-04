//! Serving one terminal data channel.
//!
//! A connection reaches here only after its hello redeemed an attach token.
//! From that point the transition is one way: this connection carries terminal
//! frames and never RPC again, which is why there is no multiplexing contract
//! to get wrong (ADR 0003, grill Q2).
//!
//! The daemon does not interpret input. A client encodes keystrokes from its
//! own replica's live mode bits and the daemon writes the bytes through — the
//! wire stays dumb on purpose (`ARCHITECTURE.md` §3).
//!
//! One channel is two tasks with two halves of the socket. The *subscriber
//! writer* owns the write half and the viewer it drains; the *read loop* owns
//! the read half and nothing else. Backlog has exactly one owner: the viewer's
//! delivery state in `stream.rs`, with its byte budget. There is no second
//! queue between that state and the socket, because a second queue is a
//! second, unrelated notion of "slow": the PR9 spike measured a channel closed
//! in nine of twelve sustained storms by an eight-frame queue that read
//! ordinary jitter as a client that had stopped reading (grill Q5/Q10).

use std::sync::Arc;
use std::time::Duration;

use corral_core::{CorralSessionId, RunId};
use corral_protocol::terminal::{FrameKind, TerminalFrame};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tracing::debug;

use crate::runtime::{
    Attachment, Delivery, InputRefused, PtyGeometry, RunOccurrence, SessionHandle, Viewer,
};
use crate::state::DaemonState;

/// How long the writer waits for one byte of socket progress before deciding
/// the client has stopped reading.
///
/// Counted only while there is a frame to write and nothing of it leaves; a
/// quiet channel is not a stalled one. Brief jitter is absorbed by the wait;
/// a client that reads but cannot keep up is handled by the viewer's byte
/// budget, not by this clock. Initial operational policy, not a wire
/// guarantee (grill Q10).
const NO_PROGRESS_DEADLINE: Duration = Duration::from_secs(2);

/// How long a channel that has ended waits for its last frame to leave.
///
/// The last frame is usually a channel error saying why this ended, and a
/// client that is reading should get it. A client that is *not* reading is
/// the reason many channels end this way, and waiting on one would hold the
/// connection — and its write half — open for as long as it stayed connected.
/// Same number as the deadline above by coincidence of policy, not by
/// definition: one is how long a frame may fail to move, the other how long a
/// finished channel lingers.
const FLUSH_GRACE: Duration = Duration::from_secs(2);

/// Serve a channel until the client goes away or the session does.
///
/// The read loop below never touches the socket's write half. A write awaited
/// inside the loop that also reads would stop reading this client at exactly
/// the moment the client waits for the daemon to read — both sides then wait
/// forever. So writes belong to a task of their own, which may block on the
/// socket without anyone else noticing: not the PTY reader, not the
/// authoritative screen, not another viewer, and not this client's own
/// `Input` and `Resize`.
pub async fn serve(
    reader: &mut OwnedReadHalf,
    writer: OwnedWriteHalf,
    leftover: Vec<u8>,
    session: CorralSessionId,
    run: RunId,
    state: &Arc<DaemonState>,
) {
    let Some(attachment) = attach(session, run, state).await else {
        return;
    };
    let _attached = ActiveAttachment::began(run, state);

    // Unbounded because it carries one message per client event — a resync
    // request, a refusal to relay — and never terminal output. Sending never
    // waits, so the read loop cannot be held by a writer that is.
    let (control, controlled) = tokio::sync::mpsc::unbounded_channel::<Control>();
    let writing = tokio::spawn(subscriber_writer(
        writer,
        attachment,
        controlled,
        session,
        run,
        Arc::clone(state),
    ));

    let mut writing = writing;
    tokio::select! {
        () = read_client(reader, &control, leftover, session, run, state) => {
            // The client left. Dropped so the writer finishes what it is
            // sending and ends; then waited for, but only briefly: a client
            // that is reading gets the last frame, and one that is not is
            // exactly why many channels end — waiting on it would hold the
            // connection open for as long as it stayed silently connected.
            drop(control);
            if tokio::time::timeout(FLUSH_GRACE, &mut writing)
                .await
                .is_err()
            {
                writing.abort();
            }
        }
        _ = &mut writing => {
            // The writer ended first: the client stopped reading, its socket
            // failed, or the run is gone. A channel that can say nothing more
            // is over, however long the client keeps its end open.
        }
    }
}

/// What the read loop asks of the writer. Never terminal output.
enum Control {
    /// The client discarded its replica and wants a fresh snapshot.
    Resync,
    /// Tell the client why the daemon refused, without ending its channel.
    Error(String),
}

/// Drain one viewer to one socket, for as long as both exist.
///
/// The viewer is the only backlog this channel has. When it ends — the screen
/// reshaped and opened a new epoch, this viewer fell past its byte budget, the
/// run finished — the writer rejoins the stream and sends the snapshot that
/// supersedes whatever was still queued. That snapshot is the resync barrier
/// ADR 0003 names: a client never receives an interior gap followed by deltas
/// presented as valid; it receives deltas up to a point, then a screen.
async fn subscriber_writer(
    mut writer: OwnedWriteHalf,
    mut attachment: Attachment,
    mut control: tokio::sync::mpsc::UnboundedReceiver<Control>,
    session: CorralSessionId,
    run: RunId,
    state: Arc<DaemonState>,
) {
    if !send_snapshot(&mut writer, &attachment).await {
        return;
    }
    state.with_runtime(|runtime| runtime.attention.opened(session));

    // Set once the session can no longer be rejoined. What the viewer still
    // holds is then the run's final output, and it is written out rather than
    // discarded: a process's last words are not superseded by anything.
    let mut draining = false;

    loop {
        tokio::select! {
            delivered = next_delivery(attachment.viewer.as_mut()) => {
                let Some(delivery) = delivered else {
                    // The stream ended and is fully drained.
                    if draining {
                        return;
                    }
                    match attach(session, run, &state).await {
                        Some(fresh) => {
                            attachment = fresh;
                            if !send_snapshot(&mut writer, &attachment).await {
                                return;
                            }
                        }
                        None => return,
                    }
                    continue;
                };

                // A viewer whose senders are gone holds deltas a fresh snapshot
                // will supersede. Checked before writing, so a client that is
                // behind is not made to read a stale backlog first.
                let superseded = !draining
                    && attachment.viewer.as_ref().is_some_and(Viewer::is_closed);
                if superseded {
                    match attach(session, run, &state).await {
                        Some(fresh) => {
                            attachment = fresh;
                            if !send_snapshot(&mut writer, &attachment).await {
                                return;
                            }
                            continue;
                        }
                        None => draining = true,
                    }
                }

                if !write_frame(&mut writer, &delta(delivery)).await {
                    return;
                }
            }
            command = control.recv() => match command {
                Some(Control::Resync) => {
                    // The client threw its replica away; what was queued for
                    // it is worthless to it. Rejoin and send a screen.
                    match attach(session, run, &state).await {
                        Some(fresh) => {
                            attachment = fresh;
                            if !send_snapshot(&mut writer, &attachment).await {
                                return;
                            }
                        }
                        None => return,
                    }
                }
                Some(Control::Error(refusal)) => {
                    let frame = TerminalFrame {
                        kind: FrameKind::ChannelError,
                        epoch: attachment.epoch,
                        sequence: attachment.sequence,
                        payload: refusal.into_bytes(),
                    };
                    if !write_frame(&mut writer, &frame).await {
                        return;
                    }
                }
                // The read loop ended: the client left or sent something
                // unreadable. Nothing more will be asked of this writer.
                None => return,
            }
        }
    }
}

fn delta(delivery: Delivery) -> TerminalFrame {
    TerminalFrame {
        kind: FrameKind::Delta,
        epoch: delivery.epoch,
        sequence: delivery.sequence,
        payload: delivery.bytes.to_vec(),
    }
}

/// The next delta for this viewer, or never when there is no stream to wait on.
///
/// A finished run's screen is a value: it carries no deltas, and nothing will
/// ever produce one (ADR 0007 L2). A branch that resolved immediately would
/// spin this loop; one that never resolves leaves the channel doing exactly
/// what it should — holding the final screen until the person goes away.
async fn next_delivery(viewer: Option<&mut Viewer>) -> Option<Delivery> {
    match viewer {
        Some(viewer) => viewer.recv().await,
        None => std::future::pending().await,
    }
}

/// Read the client's frames until it goes away or sends something unreadable.
async fn read_client(
    reader: &mut OwnedReadHalf,
    control: &tokio::sync::mpsc::UnboundedSender<Control>,
    leftover: Vec<u8>,
    session: CorralSessionId,
    run: RunId,
    state: &Arc<DaemonState>,
) {
    let mut serving = Serving {
        session,
        run,
        state,
        control,
        told_run_ended: false,
    };
    let mut pending = leftover;
    let mut buffer = [0_u8; 8192];

    match consume_pending(&mut pending, &mut serving).await {
        Handled::Continue => {}
        Handled::Close => return,
    }

    loop {
        let read = match reader.read(&mut buffer).await {
            Ok(0) | Err(_) => return,
            Ok(read) => read,
        };
        pending.extend_from_slice(&buffer[..read]);
        match consume_pending(&mut pending, &mut serving).await {
            Handled::Continue => {}
            Handled::Close => return,
        }
    }
}

/// One established attachment, for as long as the daemon can observe it.
///
/// The end is reported from a destructor because a channel has many ways to
/// end — a closed socket, a refused frame, a session that went away, a
/// shutdown — and an attachment that only reported its end on the tidy paths
/// would leave the log claiming someone is still watching.
///
/// Never the end of the Run: closing a surface does not terminate managed work.
struct ActiveAttachment<'a> {
    run: RunId,
    state: &'a Arc<DaemonState>,
}

impl<'a> ActiveAttachment<'a> {
    fn began(run: RunId, state: &'a Arc<DaemonState>) -> Self {
        state.observations().report(RunOccurrence::Attached {
            run,
            at: std::time::SystemTime::now(),
        });
        Self { run, state }
    }
}

impl Drop for ActiveAttachment<'_> {
    fn drop(&mut self) {
        self.state.observations().report(RunOccurrence::Detached {
            run: self.run,
            at: std::time::SystemTime::now(),
        });
    }
}

/// What one terminal data channel is serving, and what it has already said.
struct Serving<'a> {
    session: CorralSessionId,
    run: RunId,
    state: &'a Arc<DaemonState>,
    control: &'a tokio::sync::mpsc::UnboundedSender<Control>,
    /// Whether this client has been told its run ended. Said once, not per
    /// keystroke: the message is written over the final screen the person is
    /// reading, and repeating it would destroy what they attached to see.
    told_run_ended: bool,
}

/// Act on every complete frame in the buffer.
async fn consume_pending(pending: &mut Vec<u8>, serving: &mut Serving<'_>) -> Handled {
    loop {
        match TerminalFrame::decode_from_client(pending) {
            Err(error) => {
                debug!(%error, "a terminal frame could not be read");
                return Handled::Close;
            }
            Ok(None) => return Handled::Continue,
            Ok(Some((frame, consumed))) => {
                pending.drain(..consumed);
                match handle(&frame, serving).await {
                    Handled::Continue => {}
                    Handled::Close => return Handled::Close,
                }
            }
        }
    }
}

/// Whether the channel survives a frame.
enum Handled {
    Continue,
    Close,
}

/// Act on one client frame.
///
/// A resize is not answered from here. It opens a new epoch on the session's
/// stream, which ends every viewer's delivery — this client's writer notices,
/// rejoins, and sends the new screen. Answering it here as well would send
/// the same screen twice.
async fn handle(frame: &TerminalFrame, serving: &mut Serving<'_>) -> Handled {
    match frame.kind {
        kind if kind.is_skippable() => Handled::Continue,
        FrameKind::Input => {
            let bytes = frame.payload.clone();
            match ask_session(serving, move |handle| handle.write_input(bytes)).await {
                Some(Ok(())) => Handled::Continue,
                Some(Err(InputRefused::RunEnded)) => {
                    if std::mem::replace(&mut serving.told_run_ended, true) {
                        return Handled::Continue;
                    }
                    channel_error(serving, InputRefused::RunEnded.to_string())
                }
                _ => Handled::Close,
            }
        }
        FrameKind::Resize => {
            let Some(geometry) = decode_geometry(&frame.payload) else {
                return Handled::Continue;
            };
            match ask_session(serving, move |handle| handle.resize(geometry)).await {
                Some(Ok(Ok(_epoch))) => Handled::Continue,
                Some(Ok(Err(refused))) => channel_error(serving, refused.to_string()),
                _ => Handled::Close,
            }
        }
        FrameKind::ResyncRequest => {
            if serving.control.send(Control::Resync).is_ok() {
                Handled::Continue
            } else {
                Handled::Close
            }
        }
        FrameKind::Snapshot | FrameKind::Delta | FrameKind::ChannelError => Handled::Close,
        FrameKind::Unknown(_) => Handled::Continue,
    }
}

/// The handle for a session, if this token's Run is the one running.
///
/// The Run is checked, not just carried: a Session outlives its Runs, so a
/// token minted before a resume must never open the terminal of the process
/// that replaced it (grill Q2).
///
/// Taken out from under the registry lock deliberately. Everything a handle
/// can be asked waits on a screen thread, and waiting while holding that lock
/// — on the daemon's one reactor thread — puts every other connection behind
/// whatever that session happens to be doing.
fn handle_for(
    session: CorralSessionId,
    run: RunId,
    state: &Arc<DaemonState>,
) -> Option<Arc<SessionHandle>> {
    state.with_runtime(|runtime| {
        let handle = runtime.sessions.get(session)?;
        (handle.run() == run).then_some(handle)
    })?
}

/// Join the session's stream, off the reactor.
///
/// The blocking round trip happens on the blocking pool, so a screen thread
/// busy writing to a PTY cannot stall the daemon's reactor.
async fn attach(
    session: CorralSessionId,
    run: RunId,
    state: &Arc<DaemonState>,
) -> Option<Attachment> {
    let handle = handle_for(session, run, state)?;
    tokio::task::spawn_blocking(move || handle.attach().ok())
        .await
        .ok()?
}

/// Ask this channel's session something, off the reactor.
async fn ask_session<T: Send + 'static>(
    serving: &Serving<'_>,
    work: impl FnOnce(&SessionHandle) -> T + Send + 'static,
) -> Option<T> {
    let handle = handle_for(serving.session, serving.run, serving.state)?;
    tokio::task::spawn_blocking(move || work(&handle))
        .await
        .ok()
}

/// Send the snapshot an attachment carries, stamped with its own epoch.
async fn send_snapshot(writer: &mut OwnedWriteHalf, attachment: &Attachment) -> bool {
    let payload = match &attachment.snapshot {
        Ok(snapshot) => snapshot.payload().to_vec(),
        Err(error) => {
            let _ = write_frame(
                writer,
                &TerminalFrame {
                    kind: FrameKind::ChannelError,
                    epoch: attachment.epoch,
                    sequence: attachment.sequence,
                    payload: error.to_string().into_bytes(),
                },
            )
            .await;
            return false;
        }
    };
    write_frame(
        writer,
        &TerminalFrame {
            // The epoch and position the snapshot actually belongs to. A
            // snapshot labelled with an epoch the client has left is discarded
            // by a client that tracks epochs, and one labelled zero after
            // thousands of chunks reads as thousands of missed frames.
            kind: FrameKind::Snapshot,
            epoch: attachment.epoch,
            sequence: attachment.sequence,
            payload,
        },
    )
    .await
}

/// Ask the writer to tell this client why the daemon refused, without ending
/// its channel. The writer stamps it with the position it is at, so a client
/// that tracks epochs does not read the message as a frame from a screen it
/// has left.
fn channel_error(serving: &Serving<'_>, refusal: String) -> Handled {
    if serving.control.send(Control::Error(refusal)).is_ok() {
        Handled::Continue
    } else {
        Handled::Close
    }
}

/// Write one frame to the client, waiting for the socket as long as it moves.
///
/// `false` means this channel is over: the frame could not be encoded, the
/// socket failed, or no byte of it left within the deadline — the client has
/// stopped reading, which is the one thing a writer may not wait on.
async fn write_frame(writer: &mut OwnedWriteHalf, frame: &TerminalFrame) -> bool {
    let bytes = match frame.encode() {
        Ok(bytes) => bytes,
        Err(error) => {
            debug!(%error, "a terminal frame could not be encoded");
            return false;
        }
    };
    let mut rest = bytes.as_slice();
    while !rest.is_empty() {
        match tokio::time::timeout(NO_PROGRESS_DEADLINE, writer.write(rest)).await {
            Ok(Ok(written)) if written > 0 => rest = &rest[written..],
            _ => return false,
        }
    }
    true
}

fn decode_geometry(payload: &[u8]) -> Option<PtyGeometry> {
    if payload.len() < 4 {
        return None;
    }
    PtyGeometry::new(
        u16::from_be_bytes([payload[0], payload[1]]),
        u16::from_be_bytes([payload[2], payload[3]]),
    )
    .ok()
}

#[cfg(test)]
fn encode_geometry(geometry: PtyGeometry) -> Vec<u8> {
    let mut payload = Vec::with_capacity(4);
    payload.extend_from_slice(&geometry.rows().to_be_bytes());
    payload.extend_from_slice(&geometry.cols().to_be_bytes());
    payload
}

#[cfg(test)]
#[path = "terminal_channel_tests.rs"]
mod tests;
