//! The hook channel: a second versioned wire protocol, fixed by ADR 0004.
//!
//! It is one-way evidence. A shim reads the provider's hook stdin, frames one
//! message, delivers it to `corrald`'s hook endpoint, and takes one receipt.
//! Nothing that travels it may slow, gate, or steer the user's agent, and
//! nothing received over it may claim more than its source is entitled to.
//!
//! Versioned independently of the client protocol. The two share the framing
//! primitive and the envelope type and nothing else: the endpoint is a
//! separate socket with a dispatcher that serves this one method, so
//! "evidence-only" is a structural fact rather than an ACL promise
//! (ADR 0004 D2).

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The hook contract this build speaks.
///
/// One, and one for a while: additive evolution stays inside a version, and a
/// version governs breaking change (`AGENTS.md` §Protocol). A peer stating
/// anything else is dropped with diagnostics — and the relay exits 0 either
/// way, because fail-open is not conditional on being understood.
pub const HOOK_PROTOCOL_VERSION: u32 = 1;

/// The one method the hook endpoint serves.
pub const HOOK_DELIVER: &str = "hook.deliver";

/// The largest provider payload this channel carries.
///
/// A payload past it is dropped **whole** and marked, never truncated: a
/// truncated fact would be a fabricated one, and a systematic oversize must be
/// visible rather than silently missing (ADR 0004 D3).
pub const MAX_HOOK_PAYLOAD_BYTES: usize = 256 * 1024;

/// Why a delivery carries no payload. The only reason this version defines.
pub const PAYLOAD_OMITTED_OVERSIZE: &str = "oversize";

/// One hook event, as the provider produced it.
///
/// The payload travels as the provider wrote it, because the relay is
/// semantics-free: it never parses the payload, so payload drift cannot break
/// it and interpretation stays with the daemon's provider adapter
/// (ADR 0004 D3).
///
/// Text rather than an opaque byte encoding. The frame is UTF-8 by
/// construction and a hook payload is JSON, which is UTF-8 by specification,
/// so a byte sequence that cannot be carried here is one `corrald` could not
/// have parsed either. Such a payload is a definite error the relay fails open
/// on rather than a re-encoding invented for something no provider produces —
/// and never an oversize marker, which would state a reason that is not the
/// reason.
///
/// No arrival time: `corrald` stamps that itself, because freshness authority
/// belongs to the clock of the process that judges freshness.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HookDelivery {
    pub hook_protocol_version: u32,
    /// The opaque token `corrald` minted into this launch's injected hook
    /// command line. Correlation evidence, never authorization (ADR 0004 D5).
    pub launch_token: String,
    /// Which provider's ingress this is. The daemon dispatches interpretation
    /// on it and never guesses.
    pub provider: String,
    /// The build version of the relay binary that delivered this.
    ///
    /// Skew is normal: a settings file written at launch invokes whatever
    /// binary is installed by the time an event fires.
    pub shim_version: String,
    /// The provider's hook stdin, verbatim. Absent when `payload_omitted`
    /// says why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<String>,
    /// Present when the payload was dropped whole. Absent means the payload
    /// is what it says it is, never that one was dropped for a reason this
    /// build had no word for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_omitted: Option<String>,
}

/// The receipt, and the whole of it.
///
/// No fields, no instructions, ever. Protocol v1 has no blocking reply: an ack
/// that could carry a decision is a channel that can steer the user's agent,
/// and the bounded first-response lease belongs to a phase that earns
/// interaction interception (ADR 0004 D4).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct HookAck {}

impl HookAck {
    /// The wire value, built without a fallible encode.
    pub fn wire_value() -> Value {
        json!({})
    }
}

impl HookDelivery {
    /// A delivery carrying one provider payload, or marked oversize when the
    /// payload is past what this channel carries.
    ///
    /// The cap is applied here rather than at the endpoint so the decision is
    /// made once, by the only party that ever holds the whole payload.
    ///
    /// `None` for a payload this channel cannot carry at all. It is the
    /// relay's cue to fail open now: there is nothing to deliver and nothing
    /// truthful to say about it.
    #[must_use]
    pub fn new(
        launch_token: String,
        provider: String,
        shim_version: String,
        payload: &[u8],
    ) -> Option<Self> {
        let text = std::str::from_utf8(payload).ok()?;
        let (payload, payload_omitted) = if text.len() <= MAX_HOOK_PAYLOAD_BYTES {
            (Some(text.to_owned()), None)
        } else {
            (None, Some(PAYLOAD_OMITTED_OVERSIZE.to_owned()))
        };
        Some(Self {
            hook_protocol_version: HOOK_PROTOCOL_VERSION,
            launch_token,
            provider,
            shim_version,
            payload,
            payload_omitted,
        })
    }
}

#[cfg(test)]
#[path = "hook_tests.rs"]
mod tests;
