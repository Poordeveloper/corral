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
