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
use corral_protocol::terminal::{FrameKind, Sequence, TerminalFrame};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tracing::debug;

use crate::runtime::{Attachment, PtyGeometry, SessionHandle};
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
    let Some(mut attachment) = attach(session, run, state) else {
        return;
    };
    if !send_snapshot(writer, &attachment).await {
        return;
    }

    // Bytes the client sent in the same write as its hello already belong to
    // this framing, and dropping them would lose a first keystroke.
    let mut pending = leftover;
    let mut buffer = [0_u8; 8192];

    loop {
        // Output the daemon produced and frames the client sent are
        // independent; whichever is ready is handled, so a quiet client never
        // holds up a busy screen and a busy client never starves it.
        tokio::select! {
            delivered = attachment.viewer.recv() => {
                match delivered {
                    // The stream ended: the session opened a new epoch, or its
                    // screen is gone. Either way this viewer is owed a fresh
                    // snapshot rather than more deltas.
                    None => {
                        match attach(session, run, state) {
                            Some(fresh) => {
                                attachment = fresh;
                                if !send_snapshot(writer, &attachment).await {
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
                            payload: delivery.bytes,
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

                loop {
                    match TerminalFrame::decode(&pending) {
                        Err(error) => {
                            debug!(%error, "a terminal frame could not be read");
                            return;
                        }
                        Ok(None) => break,
                        Ok(Some((frame, consumed))) => {
                            pending.drain(..consumed);
                            match handle(&frame, writer, session, run, state, &mut attachment).await
                            {
                                Handled::Continue => {}
                                Handled::Close => return,
                            }
                        }
                    }
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
    session: CorralSessionId,
    run: RunId,
    state: &Arc<DaemonState>,
    attachment: &mut Attachment,
) -> Handled {
    match frame.kind {
        // A kind this build does not know is skipped, not fatal: the length
        // prefix said exactly how much to drop, and refusing would make every
        // future frame kind a breaking change.
        FrameKind::Unknown(_) => Handled::Continue,
        FrameKind::Input => match with_session(session, state, |handle| {
            handle.write_input(frame.payload.clone())
        }) {
            // A session that no longer answers cannot receive keystrokes, and
            // a channel that silently swallowed them would leave a person
            // typing into nothing.
            Some(Ok(())) => Handled::Continue,
            _ => Handled::Close,
        },
        FrameKind::Resize => {
            let Some(geometry) = decode_geometry(&frame.payload) else {
                // A size Corral will not build is ignored rather than acted
                // on; the client keeps the geometry it has.
                return Handled::Continue;
            };
            // The client asked because its own desired geometry changed. It
            // must never ask because it saw someone else's resize, or two
            // viewers of different sizes would reassert forever (grill Q6).
            match with_session(session, state, |handle| handle.resize(geometry)) {
                Some(Ok(Ok(_epoch))) => {
                    // The reflow dropped every viewer, this one included. It
                    // rejoins at the new shape with a snapshot that belongs
                    // to it.
                    match attach(session, run, state) {
                        Some(fresh) => {
                            *attachment = fresh;
                            if send_snapshot(writer, attachment).await {
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
                    let frame = TerminalFrame {
                        kind: FrameKind::ChannelError,
                        epoch: attachment.epoch,
                        sequence: Sequence(0),
                        payload: refused.to_string().into_bytes(),
                    };
                    if send(writer, &frame).await {
                        Handled::Continue
                    } else {
                        Handled::Close
                    }
                }
                _ => Handled::Close,
            }
        }
        FrameKind::ResyncRequest => match attach(session, run, state) {
            Some(fresh) => {
                *attachment = fresh;
                if send_snapshot(writer, attachment).await {
                    Handled::Continue
                } else {
                    Handled::Close
                }
            }
            None => Handled::Close,
        },
        // Frames only the daemon sends. A client sending one is confused about
        // the channel's direction, which is not something to guess about.
        FrameKind::Snapshot | FrameKind::Delta | FrameKind::ChannelError => Handled::Close,
    }
}

/// Join the session's stream, refusing if this token's Run is not the one
/// running.
///
/// The Run is checked, not just carried: a Session outlives its Runs, so a
/// token minted before a resume must never open the terminal of the process
/// that replaced it (grill Q2).
fn attach(session: CorralSessionId, run: RunId, state: &Arc<DaemonState>) -> Option<Attachment> {
    state.with_runtime(|runtime| {
        let handle = runtime.sessions.get(session)?;
        if handle.run() != run {
            return None;
        }
        handle.attach().ok()
    })?
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
                    sequence: Sequence(0),
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
            // The epoch the snapshot actually belongs to. A snapshot labelled
            // with an epoch the client has left is discarded by any client
            // that follows the rule, which is exactly the divergence the epoch
            // exists to prevent.
            epoch: attachment.epoch,
            sequence: Sequence(0),
            payload,
        },
    )
    .await
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

fn with_session<T>(
    session: CorralSessionId,
    state: &Arc<DaemonState>,
    work: impl FnOnce(&SessionHandle) -> T,
) -> Option<T> {
    state.with_runtime(|runtime| runtime.sessions.get(session).map(work))?
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
