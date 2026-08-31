//! The provider seam: the one owner of coding-agent knowledge in `corrald`.
//!
//! An enum with exhaustive dispatch, and it stays one now that two real
//! implementations have met it. The four boundaries a trait would draw are
//! named here as functions — launch construction, resume construction, hook
//! ingress interpretation, provider-specific validation — and the second
//! provider found them where the first left them rather than needing new ones
//! (grill Q5).
//!
//! No `dyn Provider`, and the friction is the point. A trait with defaults
//! lets a third provider inherit answers to questions it was never asked;
//! exhaustive matches make it face launch, evidence, identity, continuation,
//! capabilities, and failure semantics one at a time, with a compiler error
//! per unanswered question. That is integration review pressure, deliberately
//! kept (grill Q4).
//!
//! What the two implementations do *not* share is as load-bearing as what they
//! do. Claude is given a Corral-owned settings file its argv names; Codex is
//! given a launch-scoped config override and nothing on disk. So a launch is a
//! provider-owned plan — argv, plus an artifact only when that provider takes
//! one — rather than a neutral signature carrying one provider's file
//! (ADR 0009 D1).
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
pub mod codex;
pub mod launch;
pub mod reported;

use std::ffi::OsString;
use std::path::Path;
use std::time::SystemTime;

use corral_core::{ExternalId, RunId};

pub use launch::{
    InjectedSettings, InjectionFailed, LaunchScope, LaunchToken, LaunchTokens, NoRandomness,
    RelayInvocation, SharedLaunchTokens, sweep_launch_dir,
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
    /// Codex, and only the interactive TUI under a Corral-owned PTY.
    ///
    /// `codex exec`, headless batch, app-server orchestration, and CI-job
    /// semantics are out of managed scope: different lifecycle, interaction,
    /// attention, approval, and output semantics, deferred to their own phase.
    /// This variant must never be read as "every Codex surface is supported"
    /// (ADR 0009 D1, grill Q7).
    Codex,
}

impl KnownProvider {
    /// The provider name Corral uses on the wire, in bindings, and in logs.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => claude::PROVIDER,
            Self::Codex => codex::PROVIDER,
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
            codex::PROVIDER => Some(Self::Codex),
            _ => None,
        }
    }

    /// Every provider this build integrates, for an error that has to name
    /// them.
    pub const ALL: [Self; 2] = [Self::Claude, Self::Codex];

    /// Where this provider puts the payload when it invokes the relay.
    ///
    /// A measured fact about each provider's own delivery design, not a
    /// preference: Claude Code writes hook stdin, and Codex appends the
    /// notification JSON as one final argument and writes nothing at all
    /// (ADR 0009 D2, spike scenario 2).
    pub fn payload_delivery(self) -> PayloadDelivery {
        match self {
            Self::Claude => PayloadDelivery::Stdin,
            Self::Codex => PayloadDelivery::FinalArgument,
        }
    }
}

/// How a provider hands the relay a payload.
///
/// The relay is told which one it is (`corral_protocol::hook`), because a
/// relay left to discover it would wait out its interference budget on a pipe
/// the provider never opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PayloadDelivery {
    Stdin,
    FinalArgument,
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
/// There is always a fact: an event this build has no word for is
/// `Uninterpretable::UnknownEvent`, which asserts nothing — not even the
/// identity it happens to carry (ADR 0004 D3). The identity is separate and
/// may be absent, because an event can be true of the launch its token names
/// while carrying no id Corral can hold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderReport {
    /// The provider session id the payload names.
    pub identity: Option<ExternalId>,
    pub fact: AgentFactKind,
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
        KnownProvider::Codex => codex::PROGRAM,
    }
}

/// Refuse provider arguments that would compete with Corral's own injection.
///
/// Asked before anything is minted or written, because the answer is about the
/// request rather than about anything Corral has built yet.
pub fn refuse_arguments(provider: KnownProvider, args: &[String]) -> Result<(), ArgumentRefused> {
    match provider {
        KnownProvider::Claude => claude::refuse_arguments(args),
        KnownProvider::Codex => codex::refuse_arguments(args),
    }
}

/// What a managed launch is meant to do.
///
/// One value rather than two entry points, because a provider composes both
/// from the same injection and the two must never be able to disagree about
/// it: a continuation that lost the override would be a Run Corral believes it
/// is watching and is not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchIntent {
    /// A fresh session, carrying whatever arguments the caller passed.
    Fresh { args: Vec<String> },
    /// Continue the provider's own session as a new Run.
    ///
    /// No caller arguments: a continuation is Corral continuing what it
    /// already recorded, and arguments would make one Session's Runs differ in
    /// ways nothing recorded.
    Continue { external_id: ExternalId },
}

/// One provider's whole launch: the command line, and whatever it needed
/// written first.
///
/// The artifact is `Option` because a provider having one is a provider fact.
/// Claude takes a Corral-owned settings file its argv names; Codex takes a
/// launch-scoped config override and nothing on disk, so its file lifecycle
/// has nothing to own — one less artifact, not a gap (ADR 0009 D1).
pub struct ProviderLaunch {
    pub argv: Vec<OsString>,
    pub artifact: Option<InjectedSettings>,
}

/// Compose one provider's launch, writing whatever it needs written.
///
/// **Blocking.** Publishing a settings file ends in an `fsync`; its one caller
/// runs this off the reactor for that reason.
pub fn compose_launch(
    provider: KnownProvider,
    intent: &LaunchIntent,
    relay: &RelayInvocation,
    launch_dir: &Path,
    run: RunId,
) -> Result<ProviderLaunch, InjectionFailed> {
    match provider {
        KnownProvider::Claude => claude::compose_launch(intent, relay, launch_dir, run),
        KnownProvider::Codex => Ok(codex::compose_launch(intent, relay)),
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
        KnownProvider::Codex => codex::interpret(payload),
    }
}

/// Why a caller's provider arguments cannot be passed through.
///
/// One reason, because there is one: an argument that would defeat the
/// injection Corral is about to make. Everything else a person may want to
/// pass to their own agent is theirs, and what counts as defeating it is each
/// provider's own answer against its own command line.
///
/// Refused rather than dropped. Silently discarding an argument a person asked
/// for would be Corral deciding how their agent runs, and silently honouring
/// it would leave a launch Corral believes it is watching and is not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArgumentRefused {
    pub argument: String,
}

impl std::fmt::Display for ArgumentRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} is Corral's to pass: it is how Corral watches the session it starts for you",
            self.argument,
        )
    }
}

impl std::error::Error for ArgumentRefused {}

impl std::fmt::Display for KnownProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
