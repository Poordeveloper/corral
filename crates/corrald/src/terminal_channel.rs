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

use std::sync::Arc;

use corral_core::{CorralSessionId, RunId};
use corral_protocol::terminal::{FrameKind, TerminalFrame};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tracing::debug;

use crate::runtime::{Attachment, Delivery, InputRefused, PtyGeometry, SessionHandle, Viewer};
use crate::state::DaemonState;

/// Serve a channel until the client goes away or the session does.
pub async fn serve(
    reader: &mut OwnedReadHalf,
    writer: &mut OwnedWriteHalf,
    leftover: Vec<u8>,
    session: CorralSessionId,
    run: RunId,
    state: &Arc<DaemonState>,
) {
    let Some(attachment) = attach(session, run, state).await else {
        return;
    };
    if !send_snapshot(writer, &attachment).await {
        return;
    }
    let mut serving = Serving {
        session,
        run,
        state,
        attachment,
        told_run_ended: false,
    };

    // Bytes the client sent in the same write as its hello already belong to
    // this framing, and dropping them would lose a first keystroke.
    let mut pending = leftover;
    let mut buffer = [0_u8; 8192];

    // Acted on before waiting. A client that pipelines its hello with a resync
    // and then waits for the snapshot would otherwise deadlock: this loop
    // waits for a read that the client is waiting for an answer to make.
    match consume_pending(&mut pending, writer, &mut serving).await {
        Handled::Continue => {}
        Handled::Close => return,
    }

    loop {
        // Output the daemon produced and frames the client sent are
        // independent; whichever is ready is handled, so a quiet client never
        // holds up a busy screen and a busy client never starves it.
        tokio::select! {
            delivered = next_delivery(serving.attachment.viewer.as_mut()) => {
                match delivered {
                    // The stream ended: the session opened a new epoch, or its
                    // screen is gone. Either way this viewer is owed a fresh
                    // snapshot rather than more deltas.
                    None => {
                        match attach(serving.session, serving.run, serving.state).await {
                            Some(fresh) => {
                                serving.attachment = fresh;
                                if !send_snapshot(writer, &serving.attachment).await {
                                    return;
                                }
                            }
                            None => return,
                        }
                    }
                    Some(delivery) => {
                        let frame = TerminalFrame {
                            kind: FrameKind::Delta,
                            epoch: delivery.epoch,
                            sequence: delivery.sequence,
                            payload: delivery.bytes.to_vec(),
                        };
                        if !send(writer, &frame).await {
                            return;
                        }
                    }
                }
            }
            read = reader.read(&mut buffer) => {
                let read = match read {
                    Ok(0) | Err(_) => return,
                    Ok(read) => read,
                };
                pending.extend_from_slice(&buffer[..read]);

                match consume_pending(&mut pending, writer, &mut serving).await {
                    Handled::Continue => {}
                    Handled::Close => return,
                }
            }
        }
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

/// What one terminal data channel is serving, and what it has already said.
struct Serving<'a> {
    session: CorralSessionId,
    run: RunId,
    state: &'a Arc<DaemonState>,
    attachment: Attachment,
    /// Whether this client has been told its run ended. Said once, not per
    /// keystroke: the message is written over the final screen the person is
    /// reading, and repeating it would destroy what they attached to see.
    told_run_ended: bool,
}

/// Act on every complete frame in the buffer.
async fn consume_pending(
    pending: &mut Vec<u8>,
    writer: &mut OwnedWriteHalf,
    serving: &mut Serving<'_>,
) -> Handled {
    loop {
        match TerminalFrame::decode(pending) {
            Err(error) => {
                debug!(%error, "a terminal frame could not be read");
                return Handled::Close;
            }
            Ok(None) => return Handled::Continue,
            Ok(Some((frame, consumed))) => {
                pending.drain(..consumed);
                match handle(&frame, writer, serving).await {
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
async fn handle(
    frame: &TerminalFrame,
    writer: &mut OwnedWriteHalf,
    serving: &mut Serving<'_>,
) -> Handled {
    match frame.kind {
        // A kind this build does not know is skipped, not fatal: the length
        // prefix said exactly how much to drop, and refusing would make every
        // future frame kind a breaking change. The rule itself lives in the
        // protocol crate so both receivers cannot drift apart on it.
        kind if kind.is_skippable() => Handled::Continue,
        FrameKind::Input => {
            // A client cannot make the daemon hold or forward more than its
            // own limit, whatever the frame ceiling allows: that ceiling sizes
            // snapshots the daemon sends, not input a client pushes.
            if frame.payload.len() > corral_protocol::terminal::MAX_CLIENT_FRAME_BYTES {
                return Handled::Close;
            }
            let bytes = frame.payload.clone();
            match ask_session(serving, move |handle| handle.write_input(bytes)).await {
                Some(Ok(())) => Handled::Continue,
                // The run ended while this person was attached. Its final
                // screen is still in front of them and still worth reading, so
                // they are told once rather than disconnected.
                Some(Err(InputRefused::RunEnded)) => {
                    if std::mem::replace(&mut serving.told_run_ended, true) {
                        return Handled::Continue;
                    }
                    channel_error(
                        writer,
                        &serving.attachment,
                        InputRefused::RunEnded.to_string(),
                    )
                    .await
                }
                // A session that no longer answers cannot receive keystrokes,
                // and a channel that silently swallowed them would leave a
                // person typing into nothing.
                _ => Handled::Close,
            }
        }
        FrameKind::Resize => {
            let Some(geometry) = decode_geometry(&frame.payload) else {
                // A size Corral will not build is ignored rather than acted
                // on; the client keeps the geometry it has.
                return Handled::Continue;
            };
            // The client asked because its own desired geometry changed. It
            // must never ask because it saw someone else's resize, or two
            // viewers of different sizes would reassert forever (grill Q6).
            match ask_session(serving, move |handle| handle.resize(geometry)).await {
                Some(Ok(Ok(_epoch))) => {
                    // The reflow dropped every viewer, this one included. It
                    // rejoins at the new shape with a snapshot that belongs
                    // to it.
                    match attach(serving.session, serving.run, serving.state).await {
                        Some(fresh) => {
                            serving.attachment = fresh;
                            if send_snapshot(writer, &serving.attachment).await {
                                Handled::Continue
                            } else {
                                Handled::Close
                            }
                        }
                        None => Handled::Close,
                    }
                }
                // The terminal refused the size. The client is told so it can
                // stop believing its own geometry took, and the stream carries
                // on at the size the child still has.
                Some(Ok(Err(refused))) => {
                    channel_error(writer, &serving.attachment, refused.to_string()).await
                }
                _ => Handled::Close,
            }
        }
        FrameKind::ResyncRequest => {
            match attach(serving.session, serving.run, serving.state).await {
                Some(fresh) => {
                    serving.attachment = fresh;
                    if send_snapshot(writer, &serving.attachment).await {
                        Handled::Continue
                    } else {
                        Handled::Close
                    }
                }
                None => Handled::Close,
            }
        }
        // Frames only the daemon sends. A client sending one is confused about
        // the channel's direction, which is not something to guess about.
        FrameKind::Snapshot | FrameKind::Delta | FrameKind::ChannelError => Handled::Close,
        // Unreachable while every kind is either handled above or skippable;
        // stated so a new kind has to decide rather than fall through.
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
        // A screen that cannot be expressed is said out loud rather than sent
        // in pieces: a client given part of a viewport would render a screen
        // that never existed (ADR 0003 D8).
        Err(error) => {
            let _ = send(
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

    send(
        writer,
        &TerminalFrame {
            kind: FrameKind::Snapshot,
            // The epoch and position the snapshot actually belongs to. A
            // snapshot labelled with an epoch the client has left is discarded
            // by any client that follows the rule, and one labelled sequence
            // zero after thousands of chunks reads as a gap that never
            // happened.
            epoch: attachment.epoch,
            sequence: attachment.sequence,
            payload,
        },
    )
    .await
}

/// Tell this client why the daemon refused, without ending its channel.
///
/// Stamped with the attachment's own position so a client that tracks epochs
/// does not read the message as a frame from a screen it has left.
async fn channel_error(
    writer: &mut OwnedWriteHalf,
    attachment: &Attachment,
    refusal: String,
) -> Handled {
    let frame = TerminalFrame {
        kind: FrameKind::ChannelError,
        epoch: attachment.epoch,
        sequence: attachment.sequence,
        payload: refusal.into_bytes(),
    };
    if send(writer, &frame).await {
        Handled::Continue
    } else {
        Handled::Close
    }
}

async fn send(writer: &mut OwnedWriteHalf, frame: &TerminalFrame) -> bool {
    match frame.encode() {
        Ok(bytes) => writer.write_all(&bytes).await.is_ok(),
        Err(error) => {
            debug!(%error, "a terminal frame could not be encoded");
            false
        }
    }
}

/// A resize payload is two big-endian u16s: rows then columns.
///
/// A size that is not one Corral will build is `None` and the frame is
/// ignored, rather than reaching the emulator: four bytes from any attached
/// client must not be able to ask for a 65535x65535 active area, and must not
/// be able to ask for zero.
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

/// Encode a geometry the way a client does, so the decoder above is tested
/// against the format it actually meets rather than against itself.
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
