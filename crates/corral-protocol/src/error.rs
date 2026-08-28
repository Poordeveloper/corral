use std::fmt;

use serde::{Deserialize, Serialize};

/// A typed request failure carried inside a response envelope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolError {
    pub code: ErrorCode,
    /// Human-facing detail. Never parsed: behaviour hangs off `code` alone.
    pub message: String,
}

/// Why a request failed.
///
/// Unknown codes decode into `Unknown` rather than failing, so a peer that
/// learns a new failure mode does not become undecodable to an older one; the
/// raw code is kept because it is the only thing a diagnostic can report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum ErrorCode {
    /// The envelope was valid and the method is not served here. A
    /// compatibility safety net, never the way features are discovered.
    MethodNotFound,
    /// The method exists and the parameters are not ones it accepts.
    InvalidParams,
    /// A hello whose required identity fields are missing or ill-typed. Not an
    /// old peer: an old peer states a version, this one states nothing.
    MalformedHello,
    /// A legal frame sent where the connection's state does not allow it.
    ProtocolViolation,
    /// The daemon could not answer this request now and the same request may
    /// be sent again. Says nothing beyond "not now": it is not a claim about
    /// the daemon's health, and it is never an answer's content.
    Busy,
    /// A command id already names a different semantic command. Nothing was
    /// executed and nothing was changed.
    ///
    /// Its own code rather than `invalid_params`, because what a client must
    /// do about it is the opposite: retrying is what `busy` invites and what
    /// this forbids — the same id will never mean this command.
    CommandIdConflict,
    /// The agent a client asked to start is not one this daemon integrates.
    ///
    /// Its own code because what a surface does about it is its own: the
    /// daemon names the agents it knows, and only the surface knows how a
    /// person asks it for a plain command instead. Matching the daemon's
    /// sentence would make that hint drift with the wording.
    UnknownProvider,
    /// The Session named refuses the continuation, on its own state rather
    /// than on anything about the request.
    ///
    /// Its own code because `invalid_params` would send a client looking for a
    /// mistake in its request that is not there: the parameters were fine.
    /// Deliberately not a claim about permanence — one of the states it
    /// carries is a Run that is still live, which stops being true when that
    /// process exits, while others (a contested identity) never change in this
    /// phase. Which state it is stays in the message; a client that must tell
    /// them apart is what would earn the next code, and nothing does yet.
    SessionNotContinuable,
    Unknown(String),
}

impl ProtocolError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl ErrorCode {
    pub fn as_str(&self) -> &str {
        match self {
            Self::MethodNotFound => "method_not_found",
            Self::InvalidParams => "invalid_params",
            Self::MalformedHello => "malformed_hello",
            Self::ProtocolViolation => "protocol_violation",
            Self::Busy => "busy",
            Self::CommandIdConflict => "command_id_conflict",
            Self::UnknownProvider => "unknown_provider",
            Self::SessionNotContinuable => "session_not_continuable",
            Self::Unknown(raw) => raw,
        }
    }
}

impl From<String> for ErrorCode {
    fn from(raw: String) -> Self {
        match raw.as_str() {
            "method_not_found" => Self::MethodNotFound,
            "invalid_params" => Self::InvalidParams,
            "malformed_hello" => Self::MalformedHello,
            "protocol_violation" => Self::ProtocolViolation,
            "busy" => Self::Busy,
            "command_id_conflict" => Self::CommandIdConflict,
            "unknown_provider" => Self::UnknownProvider,
            "session_not_continuable" => Self::SessionNotContinuable,
            _ => Self::Unknown(raw),
        }
    }
}

impl From<ErrorCode> for String {
    fn from(code: ErrorCode) -> Self {
        match code {
            ErrorCode::Unknown(raw) => raw,
            other => other.as_str().to_owned(),
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ProtocolError {}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
