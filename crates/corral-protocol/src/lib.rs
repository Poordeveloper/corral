#![forbid(unsafe_code)]

//! Corral's wire vocabulary: protocol schemas, envelopes, and the
//! compatibility-facing representations clients and daemons exchange.
//!
//! Every type here is a compatibility surface. Absent fields mean unknown,
//! never a known negative; unknown methods, notifications, fields, and
//! discriminants each have a defined behaviour; and a shipped discriminant is
//! permanent once externally released (`AGENTS.md` §Protocol).
//!
//! The surface is deliberately only what protocol 2 actually serves: the
//! bootstrap handshake, `ping`, `session.list`, `session.new`,
//! `session.resume`, and `terminal.attach` — plus the separately versioned
//! hook channel in `hook`, which is a second protocol rather than part of
//! this one (ADR 0004). Nothing here describes subscriptions, live events, or
//! durable event streams, because a message that can be decoded from the wire
//! is wire surface whether or not anything serves it (ADR 0001, "no ghost wire
//! surface").

mod envelope;
mod error;
mod framing;
mod hello;
pub mod hook;
pub mod method;
pub mod terminal;

pub use envelope::{Frame, Notification, Outcome, Request, RequestId, Response};
pub use error::{ErrorCode, ProtocolError};
pub use framing::{
    FrameError, FrameReader, FrameWriter, FramingFault, MAX_FRAME_BYTES, decode_frame, encode_frame,
};
pub use hello::{
    ClientHello, Compatibility, ConnectionRole, PeerVersions, ServerHello, capability, compatible,
};

/// The protocol this build speaks.
///
/// 2 because `session.new`'s request contract changed: it requires a
/// `command_id`, which is not a field a peer may or may not send but a change
/// to what the request means — retry semantics, deduplication, and which
/// receipt answers a repeat. Two builds on either side of that both declaring
/// protocol 1 would agree in the handshake and disagree at the first request,
/// which is the handshake being wrong
/// (`docs/decisions/2026-08-25-protocol-2-acceptance.md`).
pub const PROTOCOL_VERSION: u32 = 2;

/// The oldest peer protocol this build can work with.
///
/// A version governs breaking change; capabilities govern additive evolution.
/// So this is the oldest peer this build can actually serve, and it is not
/// bound to the current version: a protocol 5 build whose every change since 3
/// was additive has a floor of 3.
///
/// It equals `PROTOCOL_VERSION` today for a reason particular to this version
/// rather than as a rule — protocol 1's `session.new` is a request this build
/// cannot serve, so there is no older peer left to work with.
pub const MIN_COMPATIBLE_PEER_VERSION: u32 = 2;

/// This build's half of the hello.
pub fn local_versions() -> PeerVersions {
    PeerVersions {
        protocol_version: PROTOCOL_VERSION,
        min_compatible_peer_version: MIN_COMPATIBLE_PEER_VERSION,
    }
}
