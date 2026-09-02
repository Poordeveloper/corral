//! The protocol 2 baseline: every method this version serves, and nothing else.

use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The bootstrap transition. Legal exactly once, as the first message.
pub const HELLO: &str = "hello";

/// Liveness acknowledgement. Carries no product facts by design.
pub const PING: &str = "ping";

/// The session list.
pub const SESSION_LIST: &str = "session.list";

/// Start a managed session and its first Run.
pub const SESSION_NEW: &str = "session.new";

/// Continue an existing Session's provider session as a new Run.
///
/// The wire and domain operation is NativeResume; the product verb a person
/// reads is "Continue in Corral". Two vocabularies on purpose
/// (`PRODUCT.md` §8).
pub const SESSION_RESUME: &str = "session.resume";

/// Obtain a one-time token for a terminal data channel.
pub const TERMINAL_ATTACH: &str = "terminal.attach";

/// How many sessions need the user, per class, as the daemon counts them.
pub const ATTENTION_SUMMARY: &str = "attention.summary";

/// Acknowledge one attention item — the one the client saw, by id.
pub const ATTENTION_ACKNOWLEDGE: &str = "attention.acknowledge";

/// Report what Corral's integration with a provider looks like, without
/// changing it.
pub const INTEGRATION_STATUS: &str = "integration.status";

/// Record that the user wants integration with a provider, and install it.
///
/// The daemon executes; a client never writes a provider's configuration
/// itself (ADR 0013 D1). During PR7 dogfood this is the only thing that
/// installs (grill Q2).
pub const INTEGRATION_ENABLE: &str = "integration.enable";

/// Record that the user does not want integration with a provider, and take
/// Corral's entries out.
pub const INTEGRATION_DISABLE: &str = "integration.disable";

/// Which provider an integration request is about.
///
/// A required field with no default: absence would have to mean "all of them"
/// or "the usual one", and both are answers the daemon would be inventing.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IntegrationParams {
    pub provider: String,
}

/// What an integration request answers.
///
/// `standing` is an open string — `installed`, `not-installed`, `drifted`,
/// `refused`, `repair-withheld` — read by a client the way `execution_state`
/// is: a value this build does not recognize is rendered as unknown rather
/// than refused, so a newer daemon may name a standing an older client has no
/// word for (`AGENTS.md` §Protocol).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IntegrationResult {
    pub provider: String,
    pub standing: String,
    /// Whether Corral can currently expect this provider's sessions to
    /// report. False is what puts a provider into Limited awareness.
    pub claims_delivery: bool,
    /// What happened, in the user's language, when the standing needs one:
    /// the cause of a refusal and what Corral did not do. Absent means there
    /// is nothing to explain, never that an explanation was lost.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// The file the answer is about, so a person can go and look at it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// The standings this build names. Open on the decode side; see
/// `IntegrationResult::standing`.
pub const STANDING_INSTALLED: &str = "installed";
pub const STANDING_NOT_INSTALLED: &str = "not-installed";
pub const STANDING_DRIFTED: &str = "drifted";
pub const STANDING_REFUSED: &str = "refused";
pub const STANDING_REPAIR_WITHHELD: &str = "repair-withheld";

/// `ping`'s result.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct PingResult {}

impl PingResult {
    /// The wire value, built without a fallible encode.
    ///
    /// Serializing a fixed empty struct cannot fail, but a `Result` at the
    /// call site invites an error path that only a bug could reach — and the
    /// only thing to put in it would be an error code no version declares.
    /// A round-trip test keeps this honest against the type.
    pub fn wire_value() -> Value {
        json!({})
    }
}

/// Whether Corral can currently present or control a session's terminal.
///
/// A different dimension from execution: it says what Corral can do with this
/// screen, not what the process is doing. A session whose screen cannot be
/// served may still be reliably running, so this never becomes a main status
/// and is never read as evidence that a process died
/// (`docs/decisions/2026-08-25-pr4-tui-grill.md` Q7).
///
/// Two values and no reason. The only thing a client acts on is whether
/// opening this session can work; telling attach failures apart waits until
/// there are several a person actually needs to distinguish.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalAccess {
    Available,
    Unavailable,
}

impl TerminalAccess {
    /// The wire spelling, and the only one. Serialization goes through here so
    /// the encoded value and `from_wire` cannot drift apart.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Unavailable => "unavailable",
        }
    }

    /// A wire value read back, or `None` for one this build does not know.
    ///
    /// Unknown, never `Unavailable`: a spelling nobody here understands says
    /// nothing about whether the terminal can be served.
    pub fn from_wire(value: &str) -> Option<Self> {
        match value {
            "available" => Some(Self::Available),
            "unavailable" => Some(Self::Unavailable),
            _ => None,
        }
    }
}

impl Serialize for TerminalAccess {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

/// Whatever a secondary field carried, or nothing.
///
/// The row's promises are its identity, its label, and its execution state. A
/// provider fact is decoration beside them, so a shape this build cannot read
/// — a number where a string was, an object that gained a required-looking
/// field — degrades that fact to unknown rather than taking the whole session
/// out of the list. An older peer must keep reading a list a newer daemon
/// extended (`AGENTS.md` §Protocol).
fn secondary_or_unknown<'de, D: serde::Deserializer<'de>, T: serde::de::DeserializeOwned>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    let carried = Option::<Value>::deserialize(deserializer)?;
    Ok(carried.and_then(|value| serde_json::from_value(value).ok()))
}

/// Whatever the field carried, reduced to what this build can act on.
///
/// Absent, null, a spelling this build does not know, and a value that is not
/// a string at all all decode to unknown rather than failing the item: an
/// older peer must keep reading a list a newer daemon extended
/// (`AGENTS.md` §Protocol).
fn terminal_access_or_unknown<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<TerminalAccess>, D::Error> {
    let carried = Option::<Value>::deserialize(deserializer)?;
    Ok(carried
        .as_ref()
        .and_then(Value::as_str)
        .and_then(TerminalAccess::from_wire))
}

/// What a session's agent last reported about itself.
///
/// Provider-neutral by construction: no provider event name reaches a client.
/// The daemon's provider adapter owns the translation, which is the same
/// placement law that keeps attention derivation in `corrald`
/// (ADR 0004 D3, layer 3).
///
/// A kind this build does not know decodes to `Unknown` rather than failing
/// the item, and a client renders no claim it cannot name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentEventKind {
    SessionStarted,
    TurnStarted,
    TurnEnded,
    AwaitingInput,
    SessionEnded,
    /// A kind a newer daemon named and this build has no word for. The raw
    /// spelling is kept because it is the only thing a diagnostic can report.
    Unknown(String),
}

impl AgentEventKind {
    /// The wire spelling, and the only one.
    pub fn as_str(&self) -> &str {
        match self {
            Self::SessionStarted => "session_started",
            Self::TurnStarted => "turn_started",
            Self::TurnEnded => "turn_ended",
            Self::AwaitingInput => "awaiting_input",
            Self::SessionEnded => "session_ended",
            Self::Unknown(raw) => raw,
        }
    }

    pub fn from_wire(value: &str) -> Self {
        match value {
            "session_started" => Self::SessionStarted,
            "turn_started" => Self::TurnStarted,
            "turn_ended" => Self::TurnEnded,
            "awaiting_input" => Self::AwaitingInput,
            "session_ended" => Self::SessionEnded,
            _ => Self::Unknown(value.to_owned()),
        }
    }
}

impl Serialize for AgentEventKind {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for AgentEventKind {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::from_wire(&String::deserialize(deserializer)?))
    }
}

/// The latest still-relevant fact the agent reported about itself.
///
/// Past tense, with provenance and age, and superseded by any newer fact. It
/// states what was reported and never asserts the present: main states are the
/// attention engine's to assert (ADR 0004 D7).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentEvent {
    pub kind: AgentEventKind,
    /// When the daemon observed it, in milliseconds since the Unix epoch.
    ///
    /// The daemon's clock, because the daemon is what judges freshness. A
    /// client renders age from it and derives nothing else.
    pub at_ms: i64,
}

impl AgentEvent {
    /// This event as the wire carries it, or nothing.
    ///
    /// A clock far enough out to overflow the field cannot describe an age
    /// either, and an omitted fact says unknown — which is true — where a
    /// saturated number would say something false with confidence.
    pub fn at(kind: AgentEventKind, observed_at: SystemTime) -> Option<Self> {
        let at_ms = match observed_at.duration_since(SystemTime::UNIX_EPOCH) {
            Ok(since) => i64::try_from(since.as_millis()).ok()?,
            Err(before) => -i64::try_from(before.duration().as_millis()).ok()?,
        };
        Some(Self { kind, at_ms })
    }

    /// The instant the field names.
    ///
    /// The inverse of `at`, and here beside it: a sign convention split across
    /// an encoder and a decoder in different crates is one that can be changed
    /// in one of them.
    pub fn observed_at(&self) -> SystemTime {
        let magnitude = Duration::from_millis(self.at_ms.unsigned_abs());
        if self.at_ms < 0 {
            SystemTime::UNIX_EPOCH - magnitude
        } else {
            SystemTime::UNIX_EPOCH + magnitude
        }
    }
}

/// The provider identity Corral currently stands behind for a session.
///
/// `external_id` is a **current claim**, not history: after an identity
/// contest it is omitted, meaning not currently assertable — never that no id
/// ever existed. Durable history keeps both ids and their evidence as
/// provenance, and one field never means both (ADR 0004 D8).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderFacts {
    /// Which agent product this session runs. It came from the managed launch
    /// and is not what an identity contest makes ambiguous, so it is retained
    /// through one.
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
}

/// The daemon's main-state claim for a session (`PRODUCT.md` §4), as the
/// wire spells it.
///
/// Open on the decode side: a state a newer daemon named decodes as
/// `Unrecognized` and is rendered as no claim, so a row is never lost to a
/// word this build lacks (`AGENTS.md` §Protocol).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttentionWireState {
    Working,
    NeedsYou,
    Ready,
    Unknown,
    Exited,
    Unrecognized(String),
}

impl AttentionWireState {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Working => "working",
            Self::NeedsYou => "needs_you",
            Self::Ready => "ready",
            Self::Unknown => "unknown",
            Self::Exited => "exited",
            Self::Unrecognized(raw) => raw,
        }
    }

    pub fn from_wire(value: &str) -> Self {
        match value {
            "working" => Self::Working,
            "needs_you" => Self::NeedsYou,
            "ready" => Self::Ready,
            "unknown" => Self::Unknown,
            "exited" => Self::Exited,
            _ => Self::Unrecognized(value.to_owned()),
        }
    }

    /// The claim this build can render, or `None` for a spelling it cannot
    /// name — which is no claim, never a guess at one.
    pub fn as_claim(&self) -> Option<&Self> {
        match self {
            Self::Unrecognized(_) => None,
            known => Some(known),
        }
    }
}

impl Serialize for AttentionWireState {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for AttentionWireState {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::from_wire(&String::deserialize(deserializer)?))
    }
}

/// Why a session needs the user. Two reasons in this phase; an unknown one
/// decodes as itself so a diagnostic can name it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttentionReasonWire {
    NeedsInput,
    TurnComplete,
    Unrecognized(String),
}

impl AttentionReasonWire {
    pub fn as_str(&self) -> &str {
        match self {
            Self::NeedsInput => "needs_input",
            Self::TurnComplete => "turn_complete",
            Self::Unrecognized(raw) => raw,
        }
    }

    pub fn from_wire(value: &str) -> Self {
        match value {
            "needs_input" => Self::NeedsInput,
            "turn_complete" => Self::TurnComplete,
            _ => Self::Unrecognized(value.to_owned()),
        }
    }
}

impl Serialize for AttentionReasonWire {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for AttentionReasonWire {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::from_wire(&String::deserialize(deserializer)?))
    }
}

/// What was last reliably known, shown beneath Unknown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastKnownFacts {
    pub state: AttentionWireState,
    pub at_unix_ms: i64,
}

/// One current attention item, by the id an acknowledgement must name.
///
/// At most one per session in this phase; the list is extensible and is
/// never a history of items (grill Q23).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttentionItemFacts {
    pub attention_item_id: String,
    pub reason: AttentionReasonWire,
    pub since_unix_ms: i64,
    pub acknowledged: bool,
}

/// The attention projection for one session.
///
/// Instants are named `_unix_ms` and durations `age_ms`; nothing is named
/// ambiguously (grill Q23).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttentionFacts {
    pub state: AttentionWireState,
    /// When the main state was entered, on the daemon's clock.
    pub since_unix_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_known: Option<LastKnownFacts>,
    #[serde(default)]
    pub items: Vec<AttentionItemFacts>,
}

/// Sessions in one attention class: all of them, and those whose current
/// item nobody has acknowledged. `0 <= unacknowledged <= total`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttentionCount {
    pub total: u32,
    pub unacknowledged: u32,
}

/// `attention.summary`'s result: a projection of the current items, never
/// a state of its own. A header shows totals; a badge shows unacknowledged;
/// no client recomputes either from a filtered list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttentionSummaryResult {
    pub needs_you: AttentionCount,
    pub ready: AttentionCount,
}

/// `attention.acknowledge`'s parameters.
///
/// The item id is required: an acknowledgement of "whatever is current"
/// would let a delayed one eat the next real blocker (grill Q18). No command
/// id, because nothing durable is recorded and the same id acknowledged twice
/// is the same result.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AttentionAcknowledgeParams {
    pub session_id: String,
    pub attention_item_id: String,
}

/// One session in a listing.
///
/// The first concrete shape the wire commits to. Every field is a promise
/// somebody has to keep: an identity, a label, what the daemon can currently
/// claim about execution, and whether Corral can serve the terminal behind it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionListItem {
    /// The Corral-owned identity. The only field a client may key on.
    pub session_id: String,
    /// A human-readable, non-authoritative display label chosen by Corral.
    ///
    /// Never parsed, never used for identity or control. Where it comes from
    /// may change — user naming, a provider-derived title — without the
    /// field's meaning changing.
    pub title: String,
    /// `running`, `exited`, or `unknown`.
    ///
    /// `unknown` says Corral cannot currently make a reliable execution claim.
    /// It is the execution dimension's own value: not an assurance, not the
    /// attention model's unknown, and never a stand-in for a process whose
    /// fate the daemon has not established. A value a peer does not recognise
    /// is treated as `unknown` rather than guessed at.
    pub execution_state: String,
    /// Whether this session's terminal can be served right now.
    ///
    /// `None` is unknown, and unknown is not a refusal: a client that could
    /// not read this still offers Open and reports whatever answer comes back,
    /// rather than disabling the one action a row has on a value it did not
    /// understand.
    #[serde(
        default,
        deserialize_with = "terminal_access_or_unknown",
        skip_serializing_if = "Option::is_none"
    )]
    pub terminal_access: Option<TerminalAccess>,
    /// The provider identity behind this session, when Corral has one.
    ///
    /// Absent is unknown — this daemon has learned nothing about a provider
    /// here — and never "there is no provider". Assurance is deliberately not
    /// carried: every provider fact in this phase rides the Attested launch
    /// channel, and a field for it would be one a later phase has to make
    /// mean something.
    #[serde(
        default,
        deserialize_with = "secondary_or_unknown",
        skip_serializing_if = "Option::is_none"
    )]
    pub provider: Option<ProviderFacts>,
    /// The latest still-relevant fact the agent reported. Absent means none.
    #[serde(
        default,
        deserialize_with = "secondary_or_unknown",
        skip_serializing_if = "Option::is_none"
    )]
    pub agent_event: Option<AgentEvent>,
    /// Where this session came from, when Corral reliably knows.
    ///
    /// An open string — `managed`, `discovered` — read the way
    /// `execution_state` is: a value this build has no word for is rendered as
    /// unknown rather than refused. Absent means Corral does not reliably know
    /// the origin, never that there is no origin: a guessed one would be the
    /// "never a guessed terminal host" rule broken by a different name
    /// (`PRODUCT.md` §8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// The working directory the session reported, when it reported one.
    ///
    /// A display hint and nothing else. It is never an identity input and
    /// never correlates two sessions: cwd and time correlation never bind
    /// (`ARCHITECTURE.md` §1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location_hint: Option<String>,
    /// The daemon's attention claim, when this daemon makes one.
    ///
    /// Absent is an older daemon, and the client renders what it rendered
    /// before this field existed — Exited or Unknown from execution state —
    /// rather than a claim of its own (ADR 0015 D1). A shape this build
    /// cannot read degrades to the same, never losing the row.
    #[serde(
        default,
        deserialize_with = "secondary_or_unknown",
        skip_serializing_if = "Option::is_none"
    )]
    pub attention: Option<AttentionFacts>,
}

/// The origins this build names. Open on the decode side; see
/// `SessionListItem::origin`.
pub const ORIGIN_MANAGED: &str = "managed";
pub const ORIGIN_DISCOVERED: &str = "discovered";

/// `session.list`'s result.
///
/// Elements stay `Value` on the decode side so a future daemon may add fields
/// without an older peer refusing the whole list.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SessionListResult {
    pub sessions: Vec<Value>,
}

/// `session.new`'s parameters.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionNewParams {
    /// The client's own id for this mutation, unique in the node's durable
    /// command namespace.
    ///
    /// Required, and required from the first version that serves a mutation:
    /// without it a lost response makes a client retry, and the retry starts a
    /// second agent that nobody asked for and nobody knows about. A UUID is
    /// the recommended form; correctness rests on the fingerprint rather than
    /// on UUIDs never colliding (ADR 0002, Q13).
    ///
    /// A retry repeats `argv` and `cwd` unchanged: one id means one semantic
    /// command, so the same id carrying different ones is a conflict rather
    /// than a retry. The geometry below is not part of that identity, so a
    /// retry sent from a terminal that has since been resized is still a
    /// retry — it replays, and the session keeps the size its first execution
    /// was given.
    pub command_id: String,
    /// The program and its arguments, for the raw runtime harness. Never
    /// joined into a display label.
    ///
    /// Mutually exclusive with `provider`: one names a command to run, the
    /// other names an agent Corral composes a command for, and a request
    /// carrying both or neither says nothing the daemon may act on. Empty
    /// where `provider` is present, which is how an older peer's request —
    /// always a non-empty `argv` — still means exactly what it always meant.
    ///
    /// Always written, empty included, and never skipped. A peer that predates
    /// `provider` requires the field, so omitting it would answer a provider
    /// launch sent to an older daemon with a decoder complaint instead of that
    /// daemon's own "this needs a command" — which is the answer absence is
    /// supposed to produce (`AGENTS.md` §Protocol).
    #[serde(default)]
    pub argv: Vec<String>,
    /// The agent product to launch, when Corral composes the command.
    ///
    /// The daemon builds the final argv, including the hook injection that
    /// makes the session attested. A client never composes one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Extra arguments passed through to the provider's own command line.
    ///
    /// Part of what the command means, so it is fingerprinted. Meaningless
    /// without `provider`, and refused there rather than ignored.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Where the program runs. Absent means the caller has no preference and
    /// the daemon supplies one — it is never silently replaced when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// The geometry the first attaching client wants.
    ///
    /// A preference, not part of what the command means: the daemon supplies a
    /// size when it is absent, and the first attach reconciles it against the
    /// terminal the person actually has. So it stays out of the command's
    /// identity — a resize between a lost response and its retry must not turn
    /// the retry into a conflict.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cols: Option<u16>,
}

/// `session.new`'s result.
///
/// It means: **the command was accepted, and a managed Run was created.** It
/// asserts nothing beyond that — not that the process is still running, not
/// that it reached the program's own code, not that it produced output. Those
/// are the Run's facts, not the command's, and they are read through
/// `session.list`'s `execution_state`.
///
/// The distinction is load-bearing because the two live on different layers.
/// A command is accepted once; a Run then runs, exits, or becomes something
/// Corral cannot establish. A caller that read this as "the process is alive"
/// would be wrong the moment it asked about `/usr/bin/true` — and would be
/// wrong in a way no additional outcome variant could fix, because the
/// question it is asking belongs to the other layer (ADR 0002 D6).
///
/// Two identities and no state field, for exactly that reason.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionNewResult {
    pub session_id: String,
    pub run_id: String,
}

/// `session.resume`'s parameters.
///
/// The Session, not the provider session: what a caller asks for is that this
/// Session runs again, and which provider identity that means is Corral's to
/// resolve from what it recorded. A caller able to name the provider id would
/// be a caller able to resume an identity Corral does not stand behind.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionResumeParams {
    /// The client's own id for this mutation. Same law as `session.new`: one
    /// id means one semantic command, and a retry replays rather than starting
    /// a second Run.
    pub command_id: String,
    pub session_id: String,
}

/// `session.resume`'s result.
///
/// The same Session, and the new Run under it. Never a new Session id: a
/// continuation is the Session it continues (ADR 0002 D1).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionResumeResult {
    pub session_id: String,
    pub run_id: String,
}

/// `terminal.attach`'s parameters.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TerminalAttachParams {
    pub session_id: String,
}

/// `terminal.attach`'s result: the token a second connection presents.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TerminalAttachResult {
    /// Single-use, short-lived, and bound to the concrete Run — not to the
    /// Session alone, which outlives it.
    pub attach_token: String,
    pub run_id: String,
    pub rows: u16,
    pub cols: u16,
}

impl SessionListResult {
    /// The wire value for the empty list, built without a fallible encode.
    pub fn empty_wire_value() -> Value {
        json!({"sessions": []})
    }
}

/// Whether `params` is acceptable for a baseline method that takes none.
///
/// A parameter this build does not implement is refused rather than dropped:
/// silently ignoring, say, a filter would answer a question nobody asked.
pub fn accepts_no_params(params: Option<&Value>) -> bool {
    matches!(params, None | Some(Value::Null))
}

#[cfg(test)]
#[path = "method_tests.rs"]
mod tests;
