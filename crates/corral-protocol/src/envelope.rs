use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::ProtocolError;

/// Correlates a response with the request that caused it.
///
/// Ids are per-connection and per-originator; PR1's daemon originates none.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RequestId(pub u64);

/// One semantic message on the wire.
///
/// The tag is a string rather than a number so an unknown kind is legible in a
/// log; an unknown kind still fails the envelope, because a peer that speaks a
/// fourth kind is speaking a contract this version never negotiated, and
/// ignoring it would present as a hang to whoever expected an answer.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Frame {
    Request(Request),
    Response(Response),
    Notification(Notification),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {
    pub id: RequestId,
    pub method: String,
    /// Absent means the request carries no parameters. It never means an empty
    /// parameter object a method may quietly reinterpret.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Notification {
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Response {
    pub id: RequestId,
    pub outcome: Outcome,
}

/// A response is either a result or an error, never both and never neither.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Result(Value),
    Error(ProtocolError),
}

impl Frame {
    pub fn request(id: RequestId, method: impl Into<String>, params: Option<Value>) -> Self {
        Self::Request(Request {
            id,
            method: method.into(),
            params,
        })
    }

    pub fn result(id: RequestId, result: Value) -> Self {
        Self::Response(Response {
            id,
            outcome: Outcome::Result(result),
        })
    }

    pub fn error(id: RequestId, error: ProtocolError) -> Self {
        Self::Response(Response {
            id,
            outcome: Outcome::Error(error),
        })
    }
}

#[cfg(test)]
#[path = "envelope_tests.rs"]
mod tests;
