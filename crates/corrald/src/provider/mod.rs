//! The provider seam: the one owner of coding-agent knowledge in `corrald`.
//!
//! A module rather than a trait, deliberately. One implementation cannot show
//! what two share, and a trait guessed from one would be a shape PR6 has to
//! either satisfy or dismantle. The boundaries a trait would eventually draw
//! are named here as functions instead — launch construction, resume
//! construction, hook ingress interpretation, provider-specific validation —
//! so the extraction, when it happens, moves code rather than inventing it
//! (grill Q5).
//!
//! This is layer 2 of ADR 0004 D3. Provider-specific ingress comes in,
//! normalized facts go out, and no provider event name is ever visible to a
//! client:
//!
//! ```text
//! provider hook wire   provider-specific facts, verbatim
//!         ↓
//! here                 raw event → normalized fact
//!         ↓
//! client-facing IPC    provider-neutral Corral semantics only
//! ```

pub mod claude;
pub mod launch;
pub mod reported;

use std::time::SystemTime;

use corral_core::ExternalId;

pub use claude::ArgumentRefused;
pub use launch::{
    InjectedSettings, InjectionFailed, LaunchScope, LaunchToken, LaunchTokens, NoRandomness,
    sweep_launch_dir,
};
pub use reported::{ReportedSession, ReportedSessions};

/// A coding-agent product Corral can launch and understand.
///
/// An enum rather than a string in the daemon's interior: the wire carries a
/// name a client typed, and turning it into one of these once, at the edge, is
/// what stops an unrecognized product from reaching launch composition as a
/// plausible-looking program name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KnownProvider {
    Claude,
}

impl KnownProvider {
    /// The provider name Corral uses on the wire, in bindings, and in logs.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => claude::PROVIDER,
        }
    }

    /// The product a caller named, or `None` for one this build does not
    /// integrate.
    ///
    /// Never a guess that an unknown name might be an executable: the provider
    /// namespace and the raw-command namespace stay distinct, and the caller
    /// is told which one it wanted (grill Q6).
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            claude::PROVIDER => Some(Self::Claude),
            _ => None,
        }
    }

    /// Every provider this build integrates, for an error that has to name
    /// them.
    pub const ALL: [Self; 1] = [Self::Claude];
}

/// A fact an agent reported about itself, in Corral's own vocabulary.
///
/// Five kinds because five are what the injected events can honestly support
/// (ADR 0004 D6). None of them is a main state: they are past-tense reports
/// with provenance and age, and Working / Needs You / Ready remain the
/// attention engine's to assert (ADR 0004 D7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentFactKind {
    SessionStarted,
    TurnStarted,
    TurnEnded,
    AwaitingInput,
    SessionEnded,
}

impl AgentFactKind {
    /// The provider-neutral wire spelling.
    pub fn as_wire(self) -> corral_protocol::method::AgentEventKind {
        use corral_protocol::method::AgentEventKind;
        match self {
            Self::SessionStarted => AgentEventKind::SessionStarted,
            Self::TurnStarted => AgentEventKind::TurnStarted,
            Self::TurnEnded => AgentEventKind::TurnEnded,
            Self::AwaitingInput => AgentEventKind::AwaitingInput,
            Self::SessionEnded => AgentEventKind::SessionEnded,
        }
    }
}

/// One normalized fact, and when `corrald` observed it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentFact {
    pub kind: AgentFactKind,
    /// Stamped on arrival by the daemon: freshness authority belongs to the
    /// clock of the process that judges freshness (ADR 0004 D3).
    pub observed_at: SystemTime,
}

/// How a provider session came to be, as its start reported it.
///
/// **Diagnostics only in this phase, and deliberately so.** ADR 0004 D6 injects
/// `SessionStart` for identity *and* for start discrimination, and this is the
/// discrimination — normalized, because the provider's own spellings never
/// leave layer 2. Nothing decides on it: a contest is a contest whichever way
/// the runtime came to name a second conversation, and a phase that models
/// in-runtime switching is the one that gets to act on this (ADR 0004 D8).
///
/// What it buys today is a person being able to find out *why* continuing a
/// Session stopped being possible. `Replaced` is the common one: clearing or
/// compacting a conversation starts a new one in the same runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionOrigin {
    Startup,
    Resumed,
    Forked,
    /// The runtime deliberately moved to a new conversation — cleared,
    /// compacted, or however else a provider spells starting over in place.
    Replaced,
    /// A spelling this build has no word for. Named rather than guessed.
    Unrecognized,
}

impl SessionOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Startup => "startup",
            Self::Resumed => "resumed",
            Self::Forked => "forked",
            Self::Replaced => "replaced",
            Self::Unrecognized => "unrecognized",
        }
    }
}

/// What one provider hook event says, once the provider adapter has read it.
///
/// Both parts are optional and they are separate questions. An event may name
/// an identity without carrying a fact worth showing, and a payload this build
/// cannot make sense of carries neither — asserting nothing rather than
/// guessing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderReport {
    /// The provider session id the payload names.
    pub identity: Option<ExternalId>,
    pub fact: Option<AgentFactKind>,
    /// How the session started, when the event is a start and says so.
    /// Diagnostics; see `SessionOrigin`.
    pub origin: Option<SessionOrigin>,
}

/// Why a hook payload produced no report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Uninterpretable {
    /// Not the shape this provider's hooks have. Diagnostics, never a panic
    /// and never an invented fact (`ARCHITECTURE.md` §5).
    Malformed,
    /// A hook event name this build has no word for. Tolerated and counted;
    /// it asserts nothing, not even the identity it happens to carry
    /// (ADR 0004 D3).
    UnknownEvent,
}

/// The executable a managed launch of this provider runs.
pub fn program(provider: KnownProvider) -> &'static str {
    match provider {
        KnownProvider::Claude => claude::PROGRAM,
    }
}

/// Refuse provider arguments that would compete with Corral's own injection.
///
/// Asked before anything is minted or written, because the answer is about the
/// request rather than about anything Corral has built yet.
pub fn refuse_arguments(
    provider: KnownProvider,
    args: &[String],
) -> Result<(), claude::ArgumentRefused> {
    match provider {
        KnownProvider::Claude => claude::refuse_arguments(args),
    }
}

/// The arguments of a fresh managed session, including its hook injection.
pub fn launch_argv(
    provider: KnownProvider,
    settings: &std::path::Path,
    args: &[String],
) -> Vec<std::ffi::OsString> {
    match provider {
        KnownProvider::Claude => claude::launch_argv(settings, args),
    }
}

/// The arguments that continue the provider's own session as a new Run.
pub fn resume_argv(
    provider: KnownProvider,
    external_id: &ExternalId,
    settings: &std::path::Path,
) -> Vec<std::ffi::OsString> {
    match provider {
        KnownProvider::Claude => claude::resume_argv(external_id, settings),
    }
}

/// Read one provider's hook payload as Corral facts.
///
/// The dispatch point, and the whole of the provider knowledge a caller needs:
/// everything below it is that provider's business.
pub fn interpret(
    provider: KnownProvider,
    payload: &str,
) -> Result<ProviderReport, Uninterpretable> {
    match provider {
        KnownProvider::Claude => claude::interpret(payload),
    }
}

impl std::fmt::Display for KnownProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
