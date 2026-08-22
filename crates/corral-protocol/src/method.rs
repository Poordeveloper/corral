//! The protocol 1 baseline: every method this version serves, and nothing else.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The bootstrap transition. Legal exactly once, as the first message.
pub const HELLO: &str = "hello";

/// Liveness acknowledgement. Carries no product facts by design.
pub const PING: &str = "ping";

/// The session list. PR1 has no registry, so it is honestly empty.
pub const SESSION_LIST: &str = "session.list";

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

/// `session.list`'s result.
///
/// The element type is deliberately unassigned: PR1 serves no sessions, and
/// giving a session an encoding here would commit the wire to a shape the
/// phase that owns sessions has not decided yet. Older peers therefore decode
/// a future daemon's sessions without claiming to understand them.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SessionListResult {
    pub sessions: Vec<Value>,
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
