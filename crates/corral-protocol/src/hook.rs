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

/// How an injected hook configuration invokes the relay.
///
/// One contract with two speakers: `corrald` writes this command line into a
/// provider's settings, and the relay recognises itself by it. Skew is the
/// normal case (ADR 0004 D3) — a settings file written at one launch invokes
/// whatever binary is installed when an event fires — so a private copy on
/// either side is a contract that can drift without anything failing to
/// compile.
pub const RELAY_SUBCOMMAND: &str = "hook-relay";
pub const RELAY_PROVIDER_FLAG: &str = "--provider";
pub const RELAY_TOKEN_FLAG: &str = "--token";

/// How a provider that delivers its payload as a process argument invokes the
/// relay.
///
/// Codex appends the notification JSON as one final argument and writes
/// nothing to stdin (ADR 0009 D2), so the relay is told where to read rather
/// than left to guess: a reader that fell back to stdin on an empty argument
/// would park on a pipe nobody writes until its deadline, spending the
/// interference budget of every event on discovering the same thing.
///
/// Skew law applies unchanged and in both directions. An older relay meeting
/// this flag ignores it, reads an stdin that ends at once, and delivers
/// nothing; the daemon drops that delivery with diagnostics. Fail-open is
/// never conditional on being understood.
///
/// Argv delivery carries a ceiling this channel does not set and cannot see;
/// `MAX_HOOK_PAYLOAD_BYTES` records it.
pub const RELAY_PAYLOAD_ARGV_FLAG: &str = "--payload-argv";

/// How a globally installed entry declares which Corral wrote it.
///
/// Ownership at global scope is structural — the relay invocation *is* the
/// owner identity (ADR 0013 D2) — and this flag is how that artifact evolves.
/// The merge engine is its only reader: an entry at a version this binary
/// understands may be upgraded in place by `repair`, and one at a newer
/// version is left alone and reported, so an older Corral never rewrites what
/// a newer Corral wrote.
///
/// The relay itself ignores it, by the same tolerance that lets an injected
/// file outlive the binary that wrote it. A version the relay understood would
/// be a second reader of a decision that has one owner.
pub const RELAY_INTEGRATION_VERSION_FLAG: &str = "--integration-version";

/// The version of the global artifact this binary writes and can upgrade.
///
/// Distinct from `HOOK_PROTOCOL_VERSION`: that versions what crosses the
/// socket, this versions what is written into a user's configuration file.
/// The two evolve for different reasons and a shared number would tie a wire
/// change to a rewrite of every installed entry.
pub const INTEGRATION_VERSION: u32 = 1;

/// The largest provider payload this channel carries.
///
/// A payload past it is dropped **whole** and marked, never truncated: a
/// truncated fact would be a fabricated one, and a systematic oversize must be
/// visible rather than silently missing (ADR 0004 D3).
///
/// It bounds what this channel carries, which is not the same as what a
/// provider can hand over. A provider that delivers its payload as a process
/// argument is bounded first by the operating system: Linux caps each single
/// argv string at `MAX_ARG_STRLEN`, 32 pages — about 128 KiB on a 4 KiB-page
/// machine — and `execve` fails with `E2BIG` before the relay process exists.
/// Past that ceiling there is no delivery *and* no marker, because the marker
/// is written by a relay that never ran. macOS has no such per-string cap and
/// reaches the 1 MiB total instead, which is why the cap is fully exercisable
/// there and not on every supported platform
/// (`docs/references/2026-08-31-pr6-codex-notify-matrix.md`).
///
/// Nothing here can repair that: Corral cannot mark what never reaches it. It
/// is recorded so that the cap is not read as a guarantee it cannot make for
/// an argv-delivering provider.
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
    ///
    /// Absent is the global scope: a globally installed entry outlives every
    /// launch and belongs to none, so there is no token to carry
    /// (ADR 0014 D1). Absence is a fact about scope and never a token that
    /// went missing — a delivery whose entry *does* carry one and whose token
    /// is unusable still names a launch, and the daemon drops that rather
    /// than reading it as external.
    ///
    /// Additive inside `hook_protocol_version 1`, and the skew is documented
    /// both ways: an older daemon meeting a token-less delivery cannot decode
    /// it and drops it with diagnostics, which is degraded awareness on a
    /// mixed pair and never interference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_token: Option<String>,
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
    /// The relay process's own pid, and its parent's.
    ///
    /// Read from the process itself, costing no parsing and nothing against
    /// the relay's budget. They are where the daemon's ancestry walk starts
    /// (ADR 0014 D2): the relay is forbidden the walk — it is short-lived and
    /// poor by contract — so it reports where it stood and the daemon does
    /// the looking.
    ///
    /// Absent means this relay did not report them, never that the process
    /// had none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay_parent_pid: Option<u32>,
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
    /// `None` only for a payload that is under the cap and is not text. It is
    /// the relay's cue to fail open now: there is nothing to deliver and
    /// nothing truthful to say about it.
    #[must_use]
    pub fn new(
        launch_token: Option<String>,
        provider: String,
        shim_version: String,
        payload: &[u8],
    ) -> Option<Self> {
        // Length first, validity second, and the order is the point. A payload
        // read up to the cap ends wherever the cap falls, which for anything
        // but ASCII is usually mid-character — so asking "is this text?" first
        // would turn every oversize payload with a non-ASCII byte near the
        // boundary into a silent drop, exactly where a systematic oversize is
        // supposed to become visible (ADR 0004 D3).
        let (payload, payload_omitted) = if payload.len() > MAX_HOOK_PAYLOAD_BYTES {
            (None, Some(PAYLOAD_OMITTED_OVERSIZE.to_owned()))
        } else {
            (Some(std::str::from_utf8(payload).ok()?.to_owned()), None)
        };
        Some(Self {
            hook_protocol_version: HOOK_PROTOCOL_VERSION,
            launch_token,
            provider,
            shim_version,
            payload,
            payload_omitted,
            relay_pid: None,
            relay_parent_pid: None,
        })
    }

    /// The same delivery, saying where the relay process stood.
    ///
    /// Separate from `new` because observing the process is the caller's to
    /// do and the caller's to skip: a relay that cannot read its own pid
    /// still delivers, and the daemon degrades the corroboration rather than
    /// losing the event.
    #[must_use]
    pub fn observed_at(mut self, pid: u32, parent_pid: u32) -> Self {
        self.relay_pid = Some(pid);
        self.relay_parent_pid = Some(parent_pid);
        self
    }

    /// The same delivery with its payload dropped and marked.
    ///
    /// The cap bounds the payload; this is for what bounds the *message*,
    /// which is what the channel actually has to carry. JSON escaping expands
    /// a control character six-fold, so a payload comfortably under the cap —
    /// pasted terminal output, say — can encode past the framing limit.
    /// Dropping it there would lose the event with no record; marking it keeps
    /// the rule that a systematic oversize is visible rather than silently
    /// missing (ADR 0004 D3).
    #[must_use]
    pub fn without_payload(&self) -> Self {
        Self {
            hook_protocol_version: self.hook_protocol_version,
            launch_token: self.launch_token.clone(),
            provider: self.provider.clone(),
            shim_version: self.shim_version.clone(),
            payload: None,
            payload_omitted: Some(PAYLOAD_OMITTED_OVERSIZE.to_owned()),
            relay_pid: self.relay_pid,
            relay_parent_pid: self.relay_parent_pid,
        }
    }
}

#[cfg(test)]
#[path = "hook_tests.rs"]
mod tests;
