//! The protocol 1 baseline: every method this version serves, and nothing else.

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

/// Obtain a one-time token for a terminal data channel.
pub const TERMINAL_ATTACH: &str = "terminal.attach";

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

/// One session in a listing.
///
/// The first concrete shape the wire commits to. Three fields, because every
/// field is a promise somebody has to keep: an identity, a label, and what the
/// daemon can currently claim about execution.
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
}

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
    /// The program and its arguments. Never joined into a display label.
    pub argv: Vec<String>,
    /// Where the program runs. Absent means the caller has no preference and
    /// the daemon supplies one — it is never silently replaced when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// The geometry the first attaching client wants.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cols: Option<u16>,
}

/// `session.new`'s result.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionNewResult {
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
