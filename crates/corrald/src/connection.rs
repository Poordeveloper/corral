use std::sync::Arc;

use corral_protocol::method::{self, PingResult, SessionListResult};
use corral_protocol::{
    ClientHello, Compatibility, ErrorCode, Frame, FrameError, FrameReader, FrameWriter,
    ProtocolError, Request, RequestId, ServerHello, compatible, local_versions,
};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::watch;
use tracing::{debug, error, warn};

use crate::lifecycle::{EstablishedGuard, Lifecycle, ShutdownReason};
use crate::policy::DaemonPolicy;
use crate::state::DaemonState;

/// What a dispatched request produced.
enum Dispatch {
    /// Answer and keep serving.
    Reply(Frame),
    /// Answer and close: the connection's state machine cannot continue.
    ReplyThenClose(Frame),
    /// The registry store declined, and is still intact. Protocol 1 defines no
    /// error code for a transient state condition, and PR2 is ruled zero-wire,
    /// so it does not mint one: this connection closes — an outcome the client
    /// already handles — and the caller may come back. The phase that serves
    /// state-derived facts on the wire owns the retryable code. The daemon
    /// keeps serving every other client meanwhile.
    CloseWithoutReply(corral_state::StateError),
    /// The registry store can no longer vouch for durable truth. Nothing is
    /// answered — a normal-looking reply from an untrusted store is the one
    /// outcome fail-closed exists to prevent — and the whole daemon stops
    /// serving (ADR 0002, Q14).
    FailClosed(corral_state::StateError),
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
    state: Arc<DaemonState>,
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

    serve_established(&mut reader, &mut writer, &mut shutdown, &lifecycle, &state).await;
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
        compatibility_result: verdict,
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
    let value = serde_json::to_value(hello).map_err(|source| {
        // Encoding a fixed struct cannot fail, so if it ever does the daemon
        // is not in a state to serve. Close, but say why: a silent close is
        // indistinguishable from the peer going away.
        error!(%source, "the server hello could not be encoded");
    })?;
    writer
        .write_frame(&Frame::result(id, value))
        .await
        .map_err(|_| ())
}

async fn serve_established(
    reader: &mut FrameReader<OwnedReadHalf>,
    writer: &mut FrameWriter<OwnedWriteHalf>,
    shutdown: &mut watch::Receiver<bool>,
    lifecycle: &Arc<Lifecycle>,
    state: &Arc<DaemonState>,
) {
    loop {
        let frame = tokio::select! {
            frame = reader.read_frame() => frame,
            _ = shutdown.changed() => return,
        };

        let dispatched = match frame {
            Ok(None) => return,
            Ok(Some(Frame::Request(request))) => dispatch(&request, state).await,
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
            Dispatch::CloseWithoutReply(error) => {
                warn!(%error, "the registry could not answer; closing the connection");
                return;
            }
            Dispatch::FailClosed(error) => {
                error!(%error, "the registry store can no longer be trusted");
                // Noted before the shutdown is committed, and separately from
                // it: a signal committing first must not turn this into a
                // clean exit.
                lifecycle.note_untrusted_state();
                lifecycle.commit_shutdown(ShutdownReason::FatalState);
                return;
            }
        };
        if writer.write_frame(&frame).await.is_err() || close {
            return;
        }
    }
}

async fn dispatch(request: &Request, state: &Arc<DaemonState>) -> Dispatch {
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
            Ok(()) => Dispatch::Reply(Frame::result(id, PingResult::wire_value())),
            Err(error) => Dispatch::Reply(Frame::error(id, error)),
        },
        method::SESSION_LIST => match no_params(request) {
            // Protocol 1 assigns no session encoding, so this build serves the
            // sessions it can describe, which is none. The registry is still
            // asked: an empty list is a claim about it, and a store that can
            // no longer vouch for durable truth must not have that claim made
            // in its name. Only that conclusion ends the daemon — a refusal
            // leaves the store intact and costs this connection alone.
            Ok(()) => match state.vouch().await {
                Ok(()) => Dispatch::Reply(Frame::result(id, SessionListResult::empty_wire_value())),
                Err(error) if error.is_fatal() => Dispatch::FailClosed(error),
                Err(error) => Dispatch::CloseWithoutReply(error),
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

fn log_frame_fault(error: &FrameError) {
    match error {
        FrameError::Io(_) => debug!("a connection ended"),
        other => debug!(%other, "closing a connection after a frame fault"),
    }
}

#[cfg(test)]
#[path = "connection_tests.rs"]
mod tests;
