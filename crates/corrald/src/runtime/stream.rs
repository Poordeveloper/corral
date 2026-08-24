//! The stream a terminal channel subscribes to: snapshots, deltas, epochs.
//!
//! One session's authoritative screen serves any number of viewers. Each gets
//! its own snapshot and then joins the same delta stream, so what two people
//! see converges without either owning the terminal (grill Q6).
//!
//! Three rules keep that honest.
//!
//! Geometry is shared session state, not per-viewer state. The last explicit
//! resize wins; a client must never send a resize merely because it received
//! one, or two viewers of different sizes would reassert forever.
//!
//! Resize opens a new epoch. Bytes recorded before a reflow cannot be replayed
//! into a screen shaped after it, so every viewer is dropped and owed a fresh
//! snapshot at the new shape.
//!
//! A slow viewer is never backpressure. Its budget is its own; when it runs
//! out, that viewer loses its stream and resyncs while the PTY, the screen,
//! and every other viewer proceed untouched.

use std::sync::Arc;

use corral_protocol::terminal::{Epoch, Sequence};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// What one viewer may have waiting before it is considered desynchronised.
///
/// Per viewer, never a shared pool: one stalled viewer must not shrink what
/// another may buffer. Initial policy default (grill mechanism defaults).
pub const SUBSCRIBER_QUEUE_BYTES: usize = 4 * 1024 * 1024;

/// Why a viewer stopped receiving.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Desynchronised {
    /// The viewer fell far enough behind that continuing would mean dropping
    /// bytes out of the middle of its stream.
    QueueOverflow,
}

/// One frame on its way to a viewer.
///
/// It carries the accounting for its own size, because only the viewer knows
/// when bytes leave its queue. Dropping this — after writing it out, or by
/// never reading it — is what returns the room.
#[derive(Debug)]
pub struct Delivery {
    pub epoch: Epoch,
    pub sequence: Sequence,
    pub bytes: Vec<u8>,
    /// Released on drop, so a viewer that keeps up keeps its whole budget and
    /// a viewer that falls behind runs out of it.
    _room: OwnedSemaphorePermit,
}

/// A viewer's end of the stream, held by whoever is writing to that client.
///
/// A tokio channel rather than a std one: the screen is filled by a plain
/// thread, which can send into it, while the side that drains it is a
/// connection task that must wait on this and on its client's bytes at once.
pub type Viewer = tokio::sync::mpsc::Receiver<Delivery>;

/// One attached viewer, from the stream's side.
struct Attached {
    /// Bounded, so a client that stops reading cannot make the daemon hold
    /// output without limit.
    outbox: tokio::sync::mpsc::Sender<Delivery>,
    /// The viewer's remaining room, in bytes.
    ///
    /// A semaphore rather than a counter this side increments: the bytes leave
    /// the queue on the viewer's side, and a number only this side touched
    /// would measure everything ever sent instead of what is still waiting —
    /// which would drop a healthy client the moment a session had produced
    /// four megabytes in total.
    room: Arc<Semaphore>,
    /// Set once this viewer can no longer be given a correct stream. Never
    /// un-set by more output: only a fresh snapshot recovers it.
    desynchronised: Option<Desynchronised>,
}

/// The authoritative sequence and epoch a session's stream is at, and the
/// viewers waiting on it.
pub struct TerminalStream {
    epoch: Epoch,
    next_sequence: Sequence,
    attached: Vec<Attached>,
}

impl TerminalStream {
    pub fn new() -> Self {
        Self {
            epoch: Epoch(0),
            next_sequence: Sequence(0),
            attached: Vec::new(),
        }
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn next_sequence(&self) -> Sequence {
        self.next_sequence
    }

    /// Record that output was appended, returning the sequence it took.
    pub fn advance(&mut self) -> Sequence {
        let sequence = self.next_sequence;
        self.next_sequence = Sequence(sequence.0 + 1);
        sequence
    }

    /// Begin a new epoch after a reflow.
    ///
    /// The sequence restarts because a sequence only means anything within the
    /// screen shape it was recorded against. Every viewer is dropped: each is
    /// owed a fresh snapshot at the new shape, and delivering pre-reflow bytes
    /// to a replica that has reflowed is the divergence the epoch prevents.
    pub fn open_epoch(&mut self) -> Epoch {
        self.epoch = Epoch(self.epoch.0 + 1);
        self.next_sequence = Sequence(0);
        self.attached.clear();
        self.epoch
    }

    /// Attach a viewer, returning the end it reads from.
    ///
    /// It joins at the stream's current position: whoever attaches has just
    /// been sent a snapshot of exactly that point.
    pub fn attach(&mut self) -> Viewer {
        // Bounded by frames at this hop; the byte budget is the semaphore.
        let (outbox, viewer) = tokio::sync::mpsc::channel(256);
        self.attached.push(Attached {
            outbox,
            room: Arc::new(Semaphore::new(SUBSCRIBER_QUEUE_BYTES)),
            desynchronised: None,
        });
        viewer
    }

    pub fn viewers(&self) -> usize {
        self.attached.len()
    }

    /// Hand output to every attached viewer.
    ///
    /// A viewer that cannot keep up loses its whole stream rather than a piece
    /// of its middle: bytes missing from the centre would render a screen that
    /// looks plausible and is wrong, which is worse than a visible resync. Its
    /// neighbours and this call are unaffected — nothing a viewer does becomes
    /// backpressure on the process producing the output.
    pub fn deliver(&mut self, sequence: Sequence, bytes: &[u8]) {
        let epoch = self.epoch;
        // A chunk larger than the whole budget could never be admitted, and
        // asking for room that cannot exist would drop every viewer. A PTY
        // read is far smaller, so this is a guard rather than a case.
        let wanted = u32::try_from(bytes.len()).unwrap_or(u32::MAX);

        for viewer in &mut self.attached {
            if viewer.desynchronised.is_some() {
                continue;
            }
            // Room is taken now and released when the viewer drops the
            // delivery, so this measures backlog rather than history.
            let Ok(room) = Arc::clone(&viewer.room).try_acquire_many_owned(wanted) else {
                viewer.desynchronised = Some(Desynchronised::QueueOverflow);
                continue;
            };
            if viewer
                .outbox
                .try_send(Delivery {
                    epoch,
                    sequence,
                    bytes: bytes.to_vec(),
                    _room: room,
                })
                .is_err()
            {
                // A full channel is the same fact as an empty budget: this
                // viewer is not keeping up. A closed one means the client
                // detached. Both end this viewer's stream.
                viewer.desynchronised = Some(Desynchronised::QueueOverflow);
            }
        }

        self.attached
            .retain(|viewer| viewer.desynchronised.is_none());
    }
}

impl Default for TerminalStream {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for TerminalStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalStream")
            .field("epoch", &self.epoch)
            .field("next_sequence", &self.next_sequence)
            .field("viewers", &self.attached.len())
            .finish()
    }
}

impl std::fmt::Display for Desynchronised {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::QueueOverflow => f.write_str("the viewer fell too far behind to continue"),
        }
    }
}

#[cfg(test)]
#[path = "stream_tests.rs"]
mod tests;
