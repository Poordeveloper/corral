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

/// The capability a hello declares to claim a terminal data channel.
pub const TERMINAL_DATA_ROLE: &str = "terminal-data";

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
    Unknown(u8),
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
            Self::Unknown(raw) => raw,
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
            other => Self::Unknown(other),
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
#[derive(Debug)]
pub enum TerminalFrameError {
    /// A frame declared a length past the safety limit.
    Oversize {
        declared: usize,
    },
    /// The peer stopped mid-frame.
    Truncated,
    Io(std::io::Error),
}

/// The fixed-size header every frame carries: kind, epoch, sequence, length.
const HEADER_BYTES: usize = 1 + 8 + 8 + 4;

impl TerminalFrame {
    pub fn encode(&self) -> Result<Vec<u8>, TerminalFrameError> {
        if self.payload.len() > MAX_TERMINAL_FRAME_BYTES {
            return Err(TerminalFrameError::Oversize {
                declared: self.payload.len(),
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
    pub fn decode(bytes: &[u8]) -> Result<Option<(Self, usize)>, TerminalFrameError> {
        if bytes.len() < HEADER_BYTES {
            return Ok(None);
        }

        let kind = FrameKind::from_byte(bytes[0]);
        let epoch = Epoch(u64::from_be_bytes(
            bytes[1..9].try_into().unwrap_or_default(),
        ));
        let sequence = Sequence(u64::from_be_bytes(
            bytes[9..17].try_into().unwrap_or_default(),
        ));
        let length = u32::from_be_bytes(bytes[17..21].try_into().unwrap_or_default()) as usize;

        if length > MAX_TERMINAL_FRAME_BYTES {
            return Err(TerminalFrameError::Oversize { declared: length });
        }
        if bytes.len() < HEADER_BYTES + length {
            return Ok(None);
        }

        Ok(Some((
            Self {
                kind,
                epoch,
                sequence,
                payload: bytes[HEADER_BYTES..HEADER_BYTES + length].to_vec(),
            },
            HEADER_BYTES + length,
        )))
    }
}

impl std::fmt::Display for TerminalFrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Oversize { declared } => write!(
                f,
                "a frame declared {declared} bytes, past the {MAX_TERMINAL_FRAME_BYTES}-byte limit"
            ),
            Self::Truncated => f.write_str("the peer closed mid-frame"),
            Self::Io(source) => write!(f, "transport failure: {source}"),
        }
    }
}

impl std::error::Error for TerminalFrameError {}

#[cfg(test)]
#[path = "terminal_tests.rs"]
mod tests;
