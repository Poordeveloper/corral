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

use corral_protocol::terminal::{Epoch, FrameKind, Sequence, TerminalFrame};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tracing::debug;

use crate::runtime::{PtyGeometry, SessionGone, SessionHandle, Subscriber};
use crate::state::DaemonState;

/// Serve a channel until the client goes away or the session does.
pub async fn serve(
    reader: &mut OwnedReadHalf,
    writer: &mut OwnedWriteHalf,
    leftover: Vec<u8>,
    session: corral_core::CorralSessionId,
    state: &Arc<DaemonState>,
) {
    // The first thing a channel owes its client is the screen: everything
    // after is relative to it.
    let Some(epoch) = send_snapshot(writer, session, state).await else {
        return;
    };
    let Some(Some(mut subscriber)) = state.with_runtime(|runtime| {
        runtime
            .sessions
            .get(session)
            .map(|_| Subscriber::joining(epoch))
    }) else {
        return;
    };

    // Bytes the client sent in the same write as its hello already belong to
    // this framing, and dropping them would lose a first keystroke.
    let mut pending = leftover;
    let mut buffer = [0_u8; 8192];
    loop {
        let read = match reader.read(&mut buffer).await {
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
                    if !handle(&frame, writer, session, state, &mut subscriber).await {
                        return;
                    }
                }
            }
        }
    }
}

/// Act on one client frame. `false` ends the channel.
async fn handle(
    frame: &TerminalFrame,
    writer: &mut OwnedWriteHalf,
    session: corral_core::CorralSessionId,
    state: &Arc<DaemonState>,
    subscriber: &mut Subscriber,
) -> bool {
    match frame.kind {
        // A kind this build does not know is skipped, not fatal: the length
        // prefix said exactly how much to drop, and refusing would make every
        // future frame kind a breaking change.
        FrameKind::Unknown(_) => true,
        FrameKind::Input => with_session(session, state, |handle| {
            handle.write_input(frame.payload.clone())
        })
        .is_some(),
        FrameKind::Resize => {
            let Some(geometry) = decode_geometry(&frame.payload) else {
                return true;
            };
            // The client asked because its own desired geometry changed. It
            // must never ask because it saw someone else's resize, or two
            // viewers of different sizes would reassert forever (grill Q6).
            let Some(Ok(epoch)) = with_session(session, state, |handle| handle.resize(geometry))
            else {
                return false;
            };
            subscriber.enter_epoch(epoch);
            send_snapshot(writer, session, state).await.is_some()
        }
        FrameKind::ResyncRequest => send_snapshot(writer, session, state).await.is_some(),
        // Frames only the daemon sends. A client sending one is confused about
        // the channel's direction, which is not something to guess about.
        FrameKind::Snapshot | FrameKind::Delta | FrameKind::ChannelError => false,
    }
}

/// Mint a snapshot and send it, returning the epoch it belongs to.
async fn send_snapshot(
    writer: &mut OwnedWriteHalf,
    session: corral_core::CorralSessionId,
    state: &Arc<DaemonState>,
) -> Option<Epoch> {
    let minted = with_session(session, state, |handle| handle.snapshot())?;
    let payload = match minted {
        Ok(Ok(snapshot)) => snapshot.payload().to_vec(),
        // A screen that cannot be expressed is said out loud rather than sent
        // in pieces: a client given part of a viewport would render a screen
        // that never existed (ADR 0003 D8).
        Ok(Err(error)) => {
            let _ = send(
                writer,
                &TerminalFrame {
                    kind: FrameKind::ChannelError,
                    epoch: Epoch(0),
                    sequence: Sequence(0),
                    payload: error.to_string().into_bytes(),
                },
            )
            .await;
            return None;
        }
        Err(SessionGone) => return None,
    };

    let epoch = Epoch(0);
    send(
        writer,
        &TerminalFrame {
            kind: FrameKind::Snapshot,
            epoch,
            sequence: Sequence(0),
            payload,
        },
    )
    .await
    .then_some(epoch)
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
    session: corral_core::CorralSessionId,
    state: &Arc<DaemonState>,
    work: impl FnOnce(&SessionHandle) -> T,
) -> Option<T> {
    state.with_runtime(|runtime| runtime.sessions.get(session).map(work))?
}

/// A resize payload is two big-endian u16s: rows then columns.
fn decode_geometry(payload: &[u8]) -> Option<PtyGeometry> {
    if payload.len() < 4 {
        return None;
    }
    Some(PtyGeometry {
        rows: u16::from_be_bytes([payload[0], payload[1]]),
        cols: u16::from_be_bytes([payload[2], payload[3]]),
    })
}

/// Encode a geometry the way a client does, so the decoder above is tested
/// against the format it actually meets rather than against itself.
#[cfg(test)]
fn encode_geometry(geometry: PtyGeometry) -> Vec<u8> {
    let mut payload = Vec::with_capacity(4);
    payload.extend_from_slice(&geometry.rows.to_be_bytes());
    payload.extend_from_slice(&geometry.cols.to_be_bytes());
    payload
}

#[cfg(test)]
#[path = "terminal_channel_tests.rs"]
mod tests;
