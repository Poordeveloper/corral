use super::*;
use corral_protocol::Outcome;
use serde_json::json;

fn request(method: &str, params: Option<serde_json::Value>) -> Request {
    Request {
        id: RequestId(9),
        method: method.to_owned(),
        params,
    }
}

fn error_code(dispatch: Dispatch) -> (ErrorCode, bool) {
    let (frame, close) = match dispatch {
        Dispatch::Reply(frame) => (frame, false),
        Dispatch::ReplyThenClose(frame) => (frame, true),
    };
    match frame {
        Frame::Response(response) => match response.outcome {
            Outcome::Error(error) => (error.code, close),
            Outcome::Result(value) => panic!("expected an error, got {value}"),
        },
        other => panic!("expected a response, got {other:?}"),
    }
}

#[test]
fn an_unknown_method_leaves_the_connection_usable() {
    let (code, close) = error_code(dispatch(&request("session.attach", None)));

    assert_eq!(code, ErrorCode::MethodNotFound);
    assert!(!close);
}

#[test]
fn a_repeated_hello_is_a_protocol_violation() {
    let (code, close) = error_code(dispatch(&request(method::HELLO, None)));

    assert_eq!(code, ErrorCode::ProtocolViolation);
    assert!(close, "the bootstrap transition happens once");
}

#[test]
fn parameters_a_baseline_method_cannot_honour_are_refused() {
    let (code, close) = error_code(dispatch(&request(
        method::SESSION_LIST,
        Some(json!({"workspace": "corral"})),
    )));

    assert_eq!(code, ErrorCode::InvalidParams);
    assert!(!close);
}

#[test]
fn the_session_list_is_empty_and_says_so() {
    let dispatched = dispatch(&request(method::SESSION_LIST, None));

    let Dispatch::Reply(Frame::Response(response)) = dispatched else {
        panic!("expected a plain reply");
    };
    match response.outcome {
        Outcome::Result(value) => assert_eq!(value, json!({"sessions": []})),
        Outcome::Error(error) => panic!("expected a result, got {error}"),
    }
}
