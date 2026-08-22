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
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use crate::framing::decode_frame;
    use serde_json::json;

    #[test]
    fn a_request_round_trips() {
        let frame = Frame::request(RequestId(7), "ping", None);
        let encoded = serde_json::to_string(&frame).expect("encode");

        assert_eq!(encoded, r#"{"type":"request","id":7,"method":"ping"}"#);

        let decoded = decode_frame(encoded.as_bytes()).expect("decode");
        match decoded {
            Frame::Request(request) => {
                assert_eq!(request.id, RequestId(7));
                assert_eq!(request.method, "ping");
                assert!(request.params.is_none());
            }
            other => panic!("expected a request, got {other:?}"),
        }
    }

    #[test]
    fn a_result_and_an_error_are_distinguishable_on_the_wire() {
        let ok = serde_json::to_string(&Frame::result(RequestId(1), json!({}))).expect("encode");
        let err = serde_json::to_string(&Frame::error(
            RequestId(1),
            ProtocolError::new(ErrorCode::MethodNotFound, "no such method"),
        ))
        .expect("encode");

        assert!(ok.contains(r#""outcome":{"result":{}}"#), "{ok}");
        assert!(err.contains(r#""outcome":{"error":"#), "{err}");
    }

    #[test]
    fn unknown_additive_fields_are_ignored() {
        let line = br#"{"type":"request","id":1,"method":"ping","trace":"abc","params":null}"#;

        let decoded = decode_frame(line).expect("tolerated");

        assert!(matches!(decoded, Frame::Request(_)));
    }

    #[test]
    fn an_unknown_frame_kind_fails_the_envelope() {
        let line = br#"{"type":"subscribe","id":1,"topic":"sessions"}"#;

        let error = decode_frame(line).expect_err("unknown kind");

        assert!(matches!(error, crate::FrameError::Envelope { .. }));
    }

    #[test]
    fn a_response_carrying_an_unknown_error_code_still_decodes() {
        let line = br#"{"type":"response","id":1,"outcome":{"error":{"code":"quota_exceeded","message":"later"}}}"#;

        let decoded = decode_frame(line).expect("decode");

        match decoded {
            Frame::Response(response) => match response.outcome {
                Outcome::Error(error) => {
                    assert_eq!(error.code, ErrorCode::Unknown("quota_exceeded".to_owned()));
                }
                other => panic!("expected an error outcome, got {other:?}"),
            },
            other => panic!("expected a response, got {other:?}"),
        }
    }
}
