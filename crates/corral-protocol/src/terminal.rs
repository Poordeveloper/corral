//! The terminal data channel: framing, and the role that claims it.
//!
//! PTY bytes never travel on the semantic RPC channel (`ARCHITECTURE.md` §3).
//! A client asks for a terminal over RPC, receives a one-time token, opens a
//! second connection to the same endpoint, and declares the terminal-data role
//! in its hello. That transition is **one way**: once a connection carries
//! terminal frames it never carries RPC again, so there is no multiplexing
//! contract to get wrong.
//!
//! The framing is binary and length-prefixed rather than newline-delimited
//! JSON, because the payload is raw PTY output — arbitrary bytes, including
//! newlines, that must reach a client's parser unmodified.

use serde::{Deserialize, Serialize};

/// The most one client-to-daemon frame may carry.
///
/// A separate number from the daemon-to-client ceiling below, because they
/// answer different questions: that one sizes a snapshot the daemon mints,
/// this one sizes what a client may make the daemon buffer and push into a
/// PTY. A keystroke, a paste, or a mouse burst is kilobytes; nothing a person
/// does needs megabytes.
pub const MAX_CLIENT_FRAME_BYTES: usize = 256 * 1024;

/// The most a terminal frame may carry.
///
/// Derived from the snapshot ceiling rather than shared with the RPC channel's
/// much smaller limit: a snapshot is the largest legitimate message on this
/// channel, so the two limits answer different questions and must not be one
/// number (ADR 0003 D8).
pub const MAX_TERMINAL_FRAME_BYTES: usize = 16 * 1024 * 1024;

/// A snapshot epoch.
///
/// Resize reflows the emulator, so bytes recorded before a resize cannot be
/// replayed into a screen shaped after it. The epoch marks which screen a
/// sequence belongs to; a client discards anything carrying an epoch it has
/// left (`ARCHITECTURE.md` §3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Epoch(pub u64);

/// A position within one epoch's byte stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Sequence(pub u64);

/// What a terminal frame carries.
///
/// The discriminant is a number because this channel is hot and its frames are
/// small; the numbers are permanent from the first tagged release exposing the
/// contract, and unknown ones have a defined behaviour rather than a fatal one
/// (AGENTS.md §Protocol).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameKind {
    /// An ANSI replay of the daemon's screen, at a sequence within an epoch.
    Snapshot,
    /// Raw PTY output, replayed unmodified.
    Delta,
    /// Bytes the client's replica encoded from a keystroke or mouse event.
    Input,
    /// The client's desired geometry.
    Resize,
    /// The client discarded its incremental state and needs a fresh snapshot.
    ResyncRequest,
    /// The daemon cannot serve this channel any further.
    ChannelError,
    /// A kind this build does not know.
    ///
    /// Carried rather than rejected: a peer that learns a new frame kind must
    /// not become undecodable to an older one, and the only thing a diagnostic
    /// can report is the number itself.
    ///
    /// The number is private so it cannot be an assigned one. An `Unknown(2)`
    /// would re-encode as `Delta` and stop being skippable, which is a frame
    /// that lies about itself.
    Unknown(UnassignedKind),
}

/// A frame-kind number this version has not assigned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnassignedKind(u8);

impl UnassignedKind {
    pub fn as_byte(self) -> u8 {
        self.0
    }
}

impl FrameKind {
    pub fn as_byte(self) -> u8 {
        match self {
            Self::Snapshot => 1,
            Self::Delta => 2,
            Self::Input => 3,
            Self::Resize => 4,
            Self::ResyncRequest => 5,
            Self::ChannelError => 6,
            Self::Unknown(raw) => raw.as_byte(),
        }
    }

    pub fn from_byte(raw: u8) -> Self {
        match raw {
            1 => Self::Snapshot,
            2 => Self::Delta,
            3 => Self::Input,
            4 => Self::Resize,
            5 => Self::ResyncRequest,
            6 => Self::ChannelError,
            other => Self::Unknown(UnassignedKind(other)),
        }
    }

    /// Whether a receiver may skip this frame and keep the channel.
    ///
    /// An unknown kind is skippable by construction — the length prefix says
    /// exactly how much to drop — which is what makes additive evolution
    /// possible on a stream that cannot be resynchronised by scanning for a
    /// delimiter.
    pub fn is_skippable(self) -> bool {
        matches!(self, Self::Unknown(_))
    }
}

/// One frame on the terminal channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalFrame {
    pub kind: FrameKind,
    pub epoch: Epoch,
    pub sequence: Sequence,
    pub payload: Vec<u8>,
}

/// Why a frame could not be read.
///
/// One variant, because there is one way this fails: a length no peer may
/// declare. A short buffer is not an error — it is `Ok(None)`, meaning the
/// rest has not arrived — and transport failures belong to whoever owns the
/// socket, not to the framing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalFrameError {
    /// A frame declared a length past the limit that applied to it.
    ///
    /// The limit is carried rather than named by a constant, because two
    /// apply: what a client may send, and what the daemon may. A message that
    /// quoted the wrong one would send a reader looking at the wrong number.
    Oversize { declared: usize, limit: usize },
}

/// The fixed-size header every frame carries: kind, epoch, sequence, length.
const HEADER_BYTES: usize = 1 + 8 + 8 + 4;

impl TerminalFrame {
    pub fn encode(&self) -> Result<Vec<u8>, TerminalFrameError> {
        if self.payload.len() > MAX_TERMINAL_FRAME_BYTES {
            return Err(TerminalFrameError::Oversize {
                declared: self.payload.len(),
                limit: MAX_TERMINAL_FRAME_BYTES,
            });
        }

        let mut bytes = Vec::with_capacity(HEADER_BYTES + self.payload.len());
        bytes.push(self.kind.as_byte());
        bytes.extend_from_slice(&self.epoch.0.to_be_bytes());
        bytes.extend_from_slice(&self.sequence.0.to_be_bytes());
        // The cast is checked above: a payload past the limit never reaches
        // here, and the limit is far below u32::MAX.
        bytes.extend_from_slice(&(self.payload.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&self.payload);
        Ok(bytes)
    }

    /// Decode one frame from a complete buffer, returning it and the bytes it
    /// consumed.
    /// Decode a frame a client sent to the daemon.
    ///
    /// Bounded by what a client may make the daemon hold, not by what the
    /// daemon may send: a header alone must not be able to reserve sixteen
    /// megabytes of buffer per connection. Named by direction rather than
    /// taking a limit, so a call site cannot pass the wrong ceiling.
    pub fn decode_from_client(bytes: &[u8]) -> Result<Option<(Self, usize)>, TerminalFrameError> {
        Self::decode_within(bytes, MAX_CLIENT_FRAME_BYTES)
    }

    /// Decode a frame the daemon sent to a client.
    ///
    /// Bounded by the snapshot ceiling, which is the largest legitimate
    /// message on this channel (ADR 0003 D8).
    pub fn decode_from_daemon(bytes: &[u8]) -> Result<Option<(Self, usize)>, TerminalFrameError> {
        Self::decode_within(bytes, MAX_TERMINAL_FRAME_BYTES)
    }

    fn decode_within(
        bytes: &[u8],
        limit: usize,
    ) -> Result<Option<(Self, usize)>, TerminalFrameError> {
        if bytes.len() < HEADER_BYTES {
            return Ok(None);
        }

        // Fixed-size reads against the header length checked just above, so
        // there is no failure to swallow: an `unwrap_or_default` here would
        // silently decode a sequence as zero if that check ever loosened.
        let (header, rest) = bytes.split_at(HEADER_BYTES);
        let kind = FrameKind::from_byte(header[0]);
        let epoch = Epoch(u64::from_be_bytes(read_eight(header, 1)));
        let sequence = Sequence(u64::from_be_bytes(read_eight(header, 9)));
        let length = u32::from_be_bytes([header[17], header[18], header[19], header[20]]) as usize;

        // Refused on the declared length, before a byte of the body is
        // waited for: the caller buffers until a frame is complete, so a
        // header alone decides how much it is asked to hold.
        if length > limit {
            return Err(TerminalFrameError::Oversize {
                declared: length,
                limit,
            });
        }
        if rest.len() < length {
            return Ok(None);
        }

        Ok(Some((
            Self {
                kind,
                epoch,
                sequence,
                payload: rest[..length].to_vec(),
            },
            HEADER_BYTES + length,
        )))
    }
}

/// Eight header bytes at a fixed offset, as an array.
///
/// The header's length is checked before this is called, so the indexing is an
/// invariant rather than a possibility.
fn read_eight(header: &[u8], at: usize) -> [u8; 8] {
    let mut eight = [0_u8; 8];
    eight.copy_from_slice(&header[at..at + 8]);
    eight
}

impl std::fmt::Display for TerminalFrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Oversize { declared, limit } => write!(
                f,
                "a frame declared {declared} bytes, past the {limit}-byte limit for its direction"
            ),
        }
    }
}

impl std::error::Error for TerminalFrameError {}

#[cfg(test)]
#[path = "terminal_tests.rs"]
mod tests;
