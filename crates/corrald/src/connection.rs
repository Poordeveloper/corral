use std::sync::Arc;

use corral_protocol::method::{
    self, PingResult, SessionListItem, SessionListResult, SessionNewParams, SessionNewResult,
    TerminalAttachParams, TerminalAttachResult,
};
use corral_protocol::{
    ClientHello, Compatibility, ConnectionRole, ErrorCode, Frame, FrameError, FrameReader,
    FrameWriter, ProtocolError, Request, RequestId, ServerHello, compatible, local_versions,
};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::watch;
use tracing::{debug, error, warn};

use crate::lifecycle::{EstablishedGuard, Lifecycle, ShutdownReason};
use crate::policy::DaemonPolicy;
use crate::runtime::{AttachGrant, AttachToken, LaunchRequest, ManagedSession, PtyGeometry};
use crate::state::{DaemonState, Vouched};

/// What a dispatched request produced.
enum Dispatch {
    /// Answer and keep serving.
    Reply(Frame),
    /// Answer and close: the connection's state machine cannot continue.
    ReplyThenClose(Frame),
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

    let (established, role) = tokio::select! {
        outcome = tokio::time::timeout(
            policy.pre_hello_deadline,
            bootstrap(&mut reader, &mut writer, &lifecycle, &state),
        ) => match outcome {
            Ok(Some(bootstrapped)) => bootstrapped,
            Ok(None) => return,
            Err(_elapsed) => {
                debug!("a pending connection did not say hello before the deadline");
                return;
            }
        },
        _ = shutdown.changed() => return,
    };

    match role {
        // A connection that redeemed a token stops being an RPC connection
        // here, permanently. There is no path back, which is what keeps the
        // two framings from ever having to share a stream.
        Some(grant) => {
            let (mut raw_reader, leftover) = reader.into_parts();
            let mut raw_writer = writer.into_inner();
            crate::terminal_channel::serve(
                &mut raw_reader,
                &mut raw_writer,
                leftover,
                grant.session,
                grant.run,
                &state,
            )
            .await;
        }
        None => {
            serve_established(&mut reader, &mut writer, &mut shutdown, &lifecycle, &state).await;
        }
    }
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
    state: &Arc<DaemonState>,
) -> Option<(EstablishedGuard, Option<AttachGrant>)> {
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

    // A hello claiming the terminal-data role is answered only if its token
    // opens something: refusing before the daemon commits to anything means a
    // spent or forged token costs a connection, not a session.
    let role = match hello.role.as_ref() {
        None => None,
        Some(role) => match redeem_role(role, state) {
            Some(grant) => Some(grant),
            None => {
                let _ = writer
                    .write_frame(&Frame::error(
                        request.id,
                        ProtocolError::new(
                            ErrorCode::ProtocolViolation,
                            "the attach token is not redeemable",
                        ),
                    ))
                    .await;
                return None;
            }
        },
    };

    // Establish before answering: between the decision and the answer, an idle
    // shutdown must not be able to commit behind this client's back.
    let guard = lifecycle.establish()?;
    if write_hello(writer, request.id, &server_hello)
        .await
        .is_err()
    {
        return None;
    }
    Some((guard, role))
}

/// Redeem a terminal-data role's token.
///
/// Redemption is one step and consumption is final: a caller whose channel
/// then fails asks for another token rather than reviving a spent one
/// (grill Q2).
fn redeem_role(role: &ConnectionRole, state: &Arc<DaemonState>) -> Option<AttachGrant> {
    let ConnectionRole::TerminalData { attach_token } = role;
    let token = AttachToken::from_wire(attach_token)?;
    state.with_runtime(|runtime| runtime.attach_tokens.redeem(&token).ok())?
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
            Dispatch::FailClosed(error) => {
                error!(%error, "the registry store can no longer be trusted");
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
            // in its name.
            //
            // Contention is answered and the connection carries on: the caller
            // learns it may send the same request again, which a closed
            // connection could not have told it. Anything else ends the
            // daemon — a read takes no lock a caller can violate, so a read
            // refused for any other reason is a store this build cannot
            // explain, and uncertainty resolves to the stricter path.
            Ok(()) => match state.vouch().await {
                Ok(Vouched::Yes) => Dispatch::Reply(session_list(id, state)),
                // A fixed message: the engine's own text names tables, columns
                // and paths, and a protocol error crosses the socket.
                Ok(Vouched::NotNow) => Dispatch::Reply(Frame::error(
                    id,
                    ProtocolError::new(ErrorCode::Busy, "the registry is held by another writer"),
                )),
                Err(error) => Dispatch::FailClosed(error),
            },
            Err(error) => Dispatch::Reply(Frame::error(id, error)),
        },
        method::SESSION_NEW => Dispatch::Reply(session_new(request, state)),
        method::TERMINAL_ATTACH => Dispatch::Reply(terminal_attach(request, state)),
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

/// The sessions this daemon runs, in the wire's first concrete session shape.
fn session_list(id: RequestId, state: &Arc<DaemonState>) -> Frame {
    let Some(described) = state.with_runtime(|runtime| runtime.sessions.describe()) else {
        return Frame::error(
            id,
            ProtocolError::new(ErrorCode::Busy, "the runtime could not be consulted"),
        );
    };

    let sessions: Vec<serde_json::Value> = described.iter().map(encode_session).collect();
    match serde_json::to_value(SessionListResult { sessions }) {
        Ok(value) => Frame::result(id, value),
        Err(source) => Frame::error(
            id,
            ProtocolError::new(ErrorCode::InvalidParams, source.to_string()),
        ),
    }
}

fn encode_session(session: &ManagedSession) -> serde_json::Value {
    serde_json::to_value(SessionListItem {
        session_id: session.session.to_string(),
        title: session.title.clone(),
        execution_state: session.execution_state.as_str().to_owned(),
    })
    .unwrap_or_else(|_| serde_json::json!({}))
}

/// Start a managed session and its first Run.
fn session_new(request: &Request, state: &Arc<DaemonState>) -> Frame {
    let id = request.id;
    let params: SessionNewParams = match request.params.clone() {
        Some(params) => match serde_json::from_value(params) {
            Ok(params) => params,
            Err(source) => {
                return Frame::error(
                    id,
                    ProtocolError::new(ErrorCode::InvalidParams, source.to_string()),
                );
            }
        },
        None => {
            return Frame::error(
                id,
                ProtocolError::new(ErrorCode::InvalidParams, "session.new needs a command"),
            );
        }
    };

    let Some((program, arguments)) = params.argv.split_first() else {
        return Frame::error(
            id,
            ProtocolError::new(ErrorCode::InvalidParams, "session.new needs a command"),
        );
    };

    // An absent working directory is the caller having no preference, so the
    // daemon supplies one. A directory the caller named and the daemon cannot
    // use is refused rather than quietly replaced.
    let working_directory = params
        .cwd
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);

    let launch = match LaunchRequest::new(
        program,
        arguments.iter().map(std::ffi::OsString::from),
        &working_directory,
    ) {
        Ok(launch) => launch,
        Err(refusal) => {
            return Frame::error(
                id,
                ProtocolError::new(ErrorCode::InvalidParams, refusal.to_string()),
            );
        }
    };

    // A size from the wire is a request, not a fact: zero rows builds a page
    // list the emulator only null-checks in debug, and an unbounded one asks
    // the daemon to allocate an active area of any size at all.
    let geometry = match PtyGeometry::new(params.rows.unwrap_or(24), params.cols.unwrap_or(80)) {
        Ok(geometry) => geometry,
        Err(impossible) => {
            return Frame::error(
                id,
                ProtocolError::new(ErrorCode::InvalidParams, impossible.to_string()),
            );
        }
    };
    let session = corral_core::CorralSessionId::mint();
    let run = corral_core::RunId::mint();

    let handle = match crate::runtime::start(&launch, geometry, session, run) {
        Ok(handle) => handle,
        // The command never ran, so no Run exists to report. Saying otherwise
        // would record a runtime occurrence that never happened.
        Err(error) => {
            return Frame::error(
                id,
                ProtocolError::new(ErrorCode::InvalidParams, error.to_string()),
            );
        }
    };

    let stored = state.with_runtime(|runtime| runtime.sessions.insert(handle));
    if stored.is_none() {
        return Frame::error(
            id,
            ProtocolError::new(ErrorCode::Busy, "the runtime could not be consulted"),
        );
    }

    match serde_json::to_value(SessionNewResult {
        session_id: session.to_string(),
        run_id: run.to_string(),
    }) {
        Ok(value) => Frame::result(id, value),
        Err(source) => Frame::error(
            id,
            ProtocolError::new(ErrorCode::InvalidParams, source.to_string()),
        ),
    }
}

/// Issue a one-time token for a terminal data channel.
fn terminal_attach(request: &Request, state: &Arc<DaemonState>) -> Frame {
    let id = request.id;
    let params: TerminalAttachParams = match request.params.clone() {
        Some(params) => match serde_json::from_value(params) {
            Ok(params) => params,
            Err(source) => {
                return Frame::error(
                    id,
                    ProtocolError::new(ErrorCode::InvalidParams, source.to_string()),
                );
            }
        },
        None => {
            return Frame::error(
                id,
                ProtocolError::new(ErrorCode::InvalidParams, "terminal.attach needs a session"),
            );
        }
    };

    let Ok(session) = params.session_id.parse::<corral_core::CorralSessionId>() else {
        return Frame::error(
            id,
            ProtocolError::new(ErrorCode::InvalidParams, "that is not a session id"),
        );
    };

    let issued = state.with_runtime(|runtime| {
        // The token names the Run, not just the Session: a Session outlives
        // its Runs, and a token that survived a resume must not open the
        // terminal of the process that replaced it (grill Q2).
        let handle = runtime.sessions.get(session)?;
        let run = handle.run();
        // The last size the screen thread published, not a question asked of
        // it: this runs on the daemon's one reactor thread while holding the
        // runtime lock, so a round trip here would block every other
        // connection behind whatever that session happens to be doing.
        let geometry = handle.last_geometry();
        let token = runtime
            .attach_tokens
            .issue(AttachGrant { session, run })
            .ok()?;
        Some((token, run, geometry))
    });

    match issued {
        Some(Some((token, run, geometry))) => match serde_json::to_value(TerminalAttachResult {
            attach_token: token.to_wire(),
            run_id: run.to_string(),
            rows: geometry.rows(),
            cols: geometry.cols(),
        }) {
            Ok(value) => Frame::result(id, value),
            Err(source) => Frame::error(
                id,
                ProtocolError::new(ErrorCode::InvalidParams, source.to_string()),
            ),
        },
        // No such session, a runtime that stopped answering, or an OS that
        // could not supply randomness. Each is a refusal to open a channel;
        // none of them says anything about a process's fate.
        Some(None) => Frame::error(
            id,
            ProtocolError::new(
                ErrorCode::InvalidParams,
                "no terminal is available for that session",
            ),
        ),
        None => Frame::error(
            id,
            ProtocolError::new(ErrorCode::Busy, "the runtime could not be consulted"),
        ),
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
