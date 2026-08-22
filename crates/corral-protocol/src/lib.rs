#![forbid(unsafe_code)]

//! Corral's wire vocabulary: protocol schemas, envelopes, and the
//! compatibility-facing representations clients and daemons exchange.
//!
//! Every type here is a compatibility surface. Absent fields mean unknown,
//! never a known negative; unknown methods, notifications, fields, and
//! discriminants each have a defined behaviour; and a shipped discriminant is
//! permanent once externally released (`AGENTS.md` §Protocol).
//!
//! The surface is deliberately only what protocol 1 actually serves: the
//! bootstrap handshake, `ping`, and `session.list`. Nothing here describes
//! subscriptions, live events, or durable event streams, because a message
//! that can be decoded from the wire is wire surface whether or not anything
//! serves it (ADR 0001, "no ghost wire surface").

mod envelope;
mod error;
mod framing;
mod hello;
pub mod method;

pub use envelope::{Frame, Notification, Outcome, Request, RequestId, Response};
pub use error::{ErrorCode, ProtocolError};
pub use framing::{
    FrameError, FrameReader, FrameWriter, FramingFault, MAX_FRAME_BYTES, decode_frame, encode_frame,
};
pub use hello::{ClientHello, Compatibility, PeerVersions, ServerHello, compatible};

/// The protocol this build speaks.
pub const PROTOCOL_VERSION: u32 = 1;

/// The oldest peer protocol this build can work with.
///
/// Protocol 1 is the first version, so it is its own floor.
pub const MIN_COMPATIBLE_PEER_VERSION: u32 = 1;

/// This build's half of the hello.
pub fn local_versions() -> PeerVersions {
    PeerVersions {
        protocol_version: PROTOCOL_VERSION,
        min_compatible_peer_version: MIN_COMPATIBLE_PEER_VERSION,
    }
}
