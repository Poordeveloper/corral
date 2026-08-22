//! The protocol 1 baseline: every method this version serves, and nothing else.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The bootstrap transition. Legal exactly once, as the first message.
pub const HELLO: &str = "hello";

/// Liveness acknowledgement. Carries no product facts by design.
pub const PING: &str = "ping";

/// The session list. PR1 has no registry, so it is honestly empty.
pub const SESSION_LIST: &str = "session.list";

/// `ping`'s result.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct PingResult {}

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

/// Whether `params` is acceptable for a baseline method that takes none.
///
/// A parameter this build does not implement is refused rather than dropped:
/// silently ignoring, say, a filter would answer a question nobody asked.
pub fn accepts_no_params(params: Option<&Value>) -> bool {
    matches!(params, None | Some(Value::Null))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_empty_session_list_encodes_as_an_empty_array() {
        let encoded = serde_json::to_string(&SessionListResult::default()).expect("encode");

        assert_eq!(encoded, r#"{"sessions":[]}"#);
    }

    #[test]
    fn a_future_session_shape_still_decodes() {
        let decoded: SessionListResult =
            serde_json::from_str(r#"{"sessions":[{"id":"s1","attention":"needs_you"}]}"#)
                .expect("decode");

        assert_eq!(decoded.sessions.len(), 1);
    }

    #[test]
    fn baseline_methods_accept_absent_and_null_params_only() {
        assert!(accepts_no_params(None));
        assert!(accepts_no_params(Some(&Value::Null)));
        assert!(!accepts_no_params(Some(&json!({"workspace": "x"}))));
    }
}
