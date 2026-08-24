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
//! into a screen shaped after it, so a client discards anything from an epoch
//! it has left and takes the fresh snapshot instead.
//!
//! A slow viewer is never backpressure. Its queue is its own; when it
//! overflows, that viewer loses its incremental state and resyncs while the
//! PTY, the screen, and every other viewer proceed untouched.

use std::collections::VecDeque;

use corral_protocol::terminal::{Epoch, Sequence};

/// What one subscriber may have queued before it is considered desynchronised.
///
/// Per subscriber, never a shared pool: one stalled viewer must not shrink
/// what another may buffer. Initial policy default (grill mechanism defaults).
pub const SUBSCRIBER_QUEUE_BYTES: usize = 4 * 1024 * 1024;

/// Why a subscriber stopped receiving.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Desynchronised {
    /// The subscriber fell far enough behind that continuing would mean
    /// dropping bytes out of the middle of its stream.
    QueueOverflow,
}

/// One viewer's position in the stream.
pub struct Subscriber {
    epoch: Epoch,
    next_sequence: Sequence,
    queued: VecDeque<Vec<u8>>,
    queued_bytes: usize,
    desynchronised: Option<Desynchronised>,
}

/// The authoritative sequence and epoch a session's stream is at.
#[derive(Debug)]
pub struct TerminalStream {
    epoch: Epoch,
    next_sequence: Sequence,
}

impl TerminalStream {
    pub fn new() -> Self {
        Self {
            epoch: Epoch(0),
            next_sequence: Sequence(0),
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
    /// screen shape it was recorded against.
    pub fn open_epoch(&mut self) -> Epoch {
        self.epoch = Epoch(self.epoch.0 + 1);
        self.next_sequence = Sequence(0);
        self.epoch
    }

    pub fn subscriber(&self) -> Subscriber {
        Subscriber {
            epoch: self.epoch,
            next_sequence: self.next_sequence,
            queued: VecDeque::new(),
            queued_bytes: 0,
            desynchronised: None,
        }
    }
}

impl Default for TerminalStream {
    fn default() -> Self {
        Self::new()
    }
}

impl Subscriber {
    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn desynchronised(&self) -> Option<Desynchronised> {
        self.desynchronised
    }

    pub fn queued_bytes(&self) -> usize {
        self.queued_bytes
    }

    /// Queue a delta for this subscriber.
    ///
    /// Over budget the subscriber is marked desynchronised and its queue is
    /// dropped whole. Never "drop the oldest and keep streaming": a viewer
    /// missing bytes out of the middle would render a screen that looks
    /// plausible and is wrong, which is worse than a visible resync.
    pub fn queue(&mut self, bytes: &[u8]) -> Result<Sequence, Desynchronised> {
        if let Some(reason) = self.desynchronised {
            return Err(reason);
        }
        if self.queued_bytes + bytes.len() > SUBSCRIBER_QUEUE_BYTES {
            self.desynchronise(Desynchronised::QueueOverflow);
            return Err(Desynchronised::QueueOverflow);
        }

        self.queued.push_back(bytes.to_vec());
        self.queued_bytes += bytes.len();
        let sequence = self.next_sequence;
        self.next_sequence = Sequence(sequence.0 + 1);
        Ok(sequence)
    }

    pub fn take_queued(&mut self) -> Option<Vec<u8>> {
        let bytes = self.queued.pop_front()?;
        self.queued_bytes -= bytes.len();
        Some(bytes)
    }

    /// Move this subscriber onto a new epoch, discarding what it had queued.
    ///
    /// Anything queued belonged to a screen shape that no longer exists;
    /// delivering it after the epoch changed would be replaying bytes into a
    /// reflowed replica, which is the divergence the epoch exists to prevent.
    pub fn enter_epoch(&mut self, epoch: Epoch) {
        self.epoch = epoch;
        self.next_sequence = Sequence(0);
        self.queued.clear();
        self.queued_bytes = 0;
        self.desynchronised = None;
    }

    /// Whether a frame from this epoch is still meaningful to this subscriber.
    pub fn accepts(&self, epoch: Epoch) -> bool {
        epoch == self.epoch
    }

    fn desynchronise(&mut self, reason: Desynchronised) {
        self.desynchronised = Some(reason);
        self.queued.clear();
        self.queued_bytes = 0;
    }
}

impl std::fmt::Debug for Subscriber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Subscriber")
            .field("epoch", &self.epoch)
            .field("queued_bytes", &self.queued_bytes)
            .field("desynchronised", &self.desynchronised)
            .finish()
    }
}

impl std::fmt::Display for Desynchronised {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::QueueOverflow => f.write_str("the subscriber fell too far behind to continue"),
        }
    }
}

#[cfg(test)]
#[path = "stream_tests.rs"]
mod tests;
