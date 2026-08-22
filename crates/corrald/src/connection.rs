use std::sync::Arc;

use corral_protocol::method::{self, PingResult, SessionListResult};
use corral_protocol::{
    ClientHello, Compatibility, ErrorCode, Frame, FrameError, FrameReader, FrameWriter,
    ProtocolError, Request, RequestId, ServerHello, compatible, local_versions,
};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::watch;
use tracing::{debug, warn};

use crate::lifecycle::{EstablishedGuard, Lifecycle};
use crate::policy::DaemonPolicy;

/// What a dispatched request produced.
enum Dispatch {
    /// Answer and keep serving.
    Reply(Frame),
    /// Answer and close: the connection's state machine cannot continue.
    ReplyThenClose(Frame),
}

/// Serve one accepted connection through its whole life.
///
/// A connection is *pending* until the handshake succeeds. Pending connections
/// have a bounded deadline and no influence on daemon lifetime, so repeatedly
/// connecting without saying hello cannot keep an otherwise idle daemon alive
/// (ADR 0001, bootstrap handshake).
pub async fn serve(
    stream: UnixStream,
    lifecycle: Arc<Lifecycle>,
    policy: DaemonPolicy,
    mut shutdown: watch::Receiver<bool>,
) {
    let (read_half, write_half) = stream.into_split();
    let mut reader = FrameReader::new(read_half);
    let mut writer = FrameWriter::new(write_half);

    let established = tokio::select! {
        outcome = tokio::time::timeout(
            policy.pre_hello_deadline,
            bootstrap(&mut reader, &mut writer, &lifecycle),
        ) => match outcome {
            Ok(Some(guard)) => guard,
            Ok(None) => return,
            Err(_elapsed) => {
                debug!("a pending connection did not say hello before the deadline");
                return;
            }
        },
        _ = shutdown.changed() => return,
    };

    serve_established(&mut reader, &mut writer, &mut shutdown).await;
    drop(established);
}

/// Run the daemon's half of the client-first hello.
///
/// `None` means the connection must be closed: it never became an established
/// client, whether because it broke the bootstrap contract, because this build
/// cannot talk to it, or because the daemon is already shutting down.
async fn bootstrap(
    reader: &mut FrameReader<OwnedReadHalf>,
    writer: &mut FrameWriter<OwnedWriteHalf>,
    lifecycle: &Arc<Lifecycle>,
) -> Option<EstablishedGuard> {
    let frame = match reader.read_frame().await {
        Ok(Some(frame)) => frame,
        Ok(None) => return None,
        Err(error) => {
            // Unparseable framing gets no typed reply: with the frame boundary
            // in doubt there is nothing to reply into.
            log_frame_fault(&error);
            return None;
        }
    };

    let request = match frame {
        Frame::Request(request) if request.method == method::HELLO => request,
        Frame::Request(request) => {
            let id = request.id;
            let _ = writer
                .write_frame(&Frame::error(
                    id,
                    ProtocolError::new(
                        ErrorCode::ProtocolViolation,
                        format!("{} is not legal before the hello", request.method),
                    ),
                ))
                .await;
            return None;
        }
        Frame::Notification(_) | Frame::Response(_) => return None,
    };

    let hello: ClientHello = match request.params {
        Some(params) => match serde_json::from_value(params) {
            Ok(hello) => hello,
            Err(source) => {
                // A malformed hello is not an old peer: an old peer states a
                // version, this one states nothing this build can compare.
                let _ = writer
                    .write_frame(&Frame::error(
                        request.id,
                        ProtocolError::new(ErrorCode::MalformedHello, source.to_string()),
                    ))
                    .await;
                return None;
            }
        },
        None => {
            let _ = writer
                .write_frame(&Frame::error(
                    request.id,
                    ProtocolError::new(
                        ErrorCode::MalformedHello,
                        "the hello carried no protocol identity",
                    ),
                ))
                .await;
            return None;
        }
    };

    let ours = local_versions();
    let theirs = hello.versions();
    let verdict = if compatible(ours, theirs) {
        Compatibility::Compatible
    } else {
        Compatibility::Incompatible
    };
    let server_hello = ServerHello {
        protocol_version: ours.protocol_version,
        min_compatible_peer_version: ours.min_compatible_peer_version,
        capabilities: Default::default(),
        compatibility: verdict,
    };

    if verdict == Compatibility::Incompatible {
        // The verdict is stated in the bootstrap envelope, carrying this
        // daemon's own versions, so the client can report facts rather than
        // guess. Then the connection closes: it never establishes.
        let _ = write_hello(writer, request.id, &server_hello).await;
        return None;
    }

    // Establish before answering: between the decision and the answer, an idle
    // shutdown must not be able to commit behind this client's back.
    let guard = lifecycle.establish()?;
    if write_hello(writer, request.id, &server_hello)
        .await
        .is_err()
    {
        return None;
    }
    Some(guard)
}

async fn write_hello(
    writer: &mut FrameWriter<OwnedWriteHalf>,
    id: RequestId,
    hello: &ServerHello,
) -> Result<(), ()> {
    let value = serde_json::to_value(hello).map_err(|_| ())?;
    writer
        .write_frame(&Frame::result(id, value))
        .await
        .map_err(|_| ())
}

async fn serve_established(
    reader: &mut FrameReader<OwnedReadHalf>,
    writer: &mut FrameWriter<OwnedWriteHalf>,
    shutdown: &mut watch::Receiver<bool>,
) {
    loop {
        let frame = tokio::select! {
            frame = reader.read_frame() => frame,
            _ = shutdown.changed() => return,
        };

        let dispatched = match frame {
            Ok(None) => return,
            Ok(Some(Frame::Request(request))) => dispatch(&request),
            // Unknown notifications are ignored by design: a notification
            // expects no answer, so dropping one cannot present as a hang.
            Ok(Some(Frame::Notification(notification))) => {
                debug!(method = %notification.method, "ignored a notification");
                continue;
            }
            // Protocol 1 daemons originate nothing, so no response can be an
            // answer to anything this daemon sent.
            Ok(Some(Frame::Response(response))) => {
                warn!(id = response.id.0, "a response arrived for no request");
                return;
            }
            Err(error) => {
                log_frame_fault(&error);
                return;
            }
        };

        let (frame, close) = match dispatched {
            Dispatch::Reply(frame) => (frame, false),
            Dispatch::ReplyThenClose(frame) => (frame, true),
        };
        if writer.write_frame(&frame).await.is_err() || close {
            return;
        }
    }
}

fn dispatch(request: &Request) -> Dispatch {
    let id = request.id;
    match request.method.as_str() {
        // The hello is a bootstrap transition, not an idempotent request.
        method::HELLO => Dispatch::ReplyThenClose(Frame::error(
            id,
            ProtocolError::new(
                ErrorCode::ProtocolViolation,
                "the connection has already completed its handshake",
            ),
        )),
        method::PING => match no_params(request) {
            Ok(()) => match serde_json::to_value(PingResult::default()) {
                Ok(value) => Dispatch::Reply(Frame::result(id, value)),
                Err(source) => Dispatch::Reply(internal_error(id, &source)),
            },
            Err(error) => Dispatch::Reply(Frame::error(id, error)),
        },
        method::SESSION_LIST => match no_params(request) {
            // PR1 owns no registry, so the honest answer is an empty list
            // rather than an absent one.
            Ok(()) => match serde_json::to_value(SessionListResult::default()) {
                Ok(value) => Dispatch::Reply(Frame::result(id, value)),
                Err(source) => Dispatch::Reply(internal_error(id, &source)),
            },
            Err(error) => Dispatch::Reply(Frame::error(id, error)),
        },
        // A compatibility safety net, not how features are discovered: the
        // connection stays usable.
        other => Dispatch::Reply(Frame::error(
            id,
            ProtocolError::new(
                ErrorCode::MethodNotFound,
                format!("this daemon speaks protocol 1 and does not serve {other}"),
            ),
        )),
    }
}

/// Baseline methods take no parameters, and parameters they do not implement
/// are refused rather than ignored: quietly dropping, say, a filter would
/// answer a question the client did not ask.
fn no_params(request: &Request) -> Result<(), ProtocolError> {
    if method::accepts_no_params(request.params.as_ref()) {
        Ok(())
    } else {
        Err(ProtocolError::new(
            ErrorCode::InvalidParams,
            format!("{} takes no parameters in protocol 1", request.method),
        ))
    }
}

fn internal_error(id: RequestId, source: &serde_json::Error) -> Frame {
    warn!(%source, "a baseline result failed to encode");
    Frame::error(
        id,
        ProtocolError::new(
            ErrorCode::Unknown("internal".to_owned()),
            source.to_string(),
        ),
    )
}

fn log_frame_fault(error: &FrameError) {
    match error {
        FrameError::Io(_) => debug!("a connection ended"),
        other => debug!(%other, "closing a connection after a frame fault"),
    }
}

#[cfg(test)]
mod tests {
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
}
