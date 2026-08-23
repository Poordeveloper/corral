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
