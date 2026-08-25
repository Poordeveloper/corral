use std::sync::Arc;
use std::time::SystemTime;

use corral_core::{Command, CommandFingerprint, CommandId, CommandKind, OccurrenceTime};
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

use crate::in_flight::{self, Claim, Concluded};
use crate::lifecycle::{EstablishedGuard, Lifecycle, ShutdownReason};
use crate::policy::DaemonPolicy;
use crate::runtime::{
    AttachGrant, AttachToken, LaunchRequest, ManagedSession, PendingSession, PtyGeometry,
};
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
            crate::terminal_channel::serve(
                &mut raw_reader,
                writer.into_inner(),
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

    // Establish before answering: between the decision and the answer, an idle
    // shutdown must not be able to commit behind this client's back.
    let guard = lifecycle.establish()?;

    // Redeemed only now. A token is single-use, so consuming it before the
    // connection is certain to be served would spend a client's capability on
    // a connection that then closed with no answer at all.
    let role = match hello.role.as_ref() {
        None => None,
        // A role this build does not serve carried no token at all, so saying
        // the token is bad would send a client looking for a problem it does
        // not have. The decode already keeps the two facts apart; so does this.
        Some(ConnectionRole::Unknown { kind }) => {
            let _ = writer
                .write_frame(&Frame::error(
                    request.id,
                    ProtocolError::new(
                        ErrorCode::InvalidParams,
                        format!("this daemon does not serve the {kind} role"),
                    ),
                ))
                .await;
            return None;
        }
        // The kind is one this build serves; what is missing is the client's
        // own field. Saying so is the difference between a client fixing its
        // bug and a client concluding the daemon is too old.
        Some(ConnectionRole::Malformed { kind }) => {
            let _ = writer
                .write_frame(&Frame::error(
                    request.id,
                    ProtocolError::new(
                        ErrorCode::InvalidParams,
                        format!("the {kind} role needs an attach token"),
                    ),
                ))
                .await;
            return None;
        }
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
    // A role this build does not serve is refused as a role, not treated as a
    // malformed hello: the client stated a version this daemon can compare and
    // asked for something it does not have.
    let ConnectionRole::TerminalData { attach_token } = role else {
        return None;
    };
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
        // A mutation must not be admitted under the condition a read is
        // refused. Without this, a daemon that cannot vouch for durable truth
        // would still fork children and mint terminal capabilities while
        // refusing to list what it had just created.
        method::SESSION_NEW => match state.vouch().await {
            Ok(Vouched::Yes) => session_new(request, state).await,
            Ok(Vouched::NotNow) => Dispatch::Reply(Frame::error(
                id,
                ProtocolError::new(ErrorCode::Busy, "the registry is held by another writer"),
            )),
            Err(error) => Dispatch::FailClosed(error),
        },
        method::TERMINAL_ATTACH => match state.vouch().await {
            Ok(Vouched::Yes) => Dispatch::Reply(terminal_attach(request, state)),
            Ok(Vouched::NotNow) => Dispatch::Reply(Frame::error(
                id,
                ProtocolError::new(ErrorCode::Busy, "the registry is held by another writer"),
            )),
            Err(error) => Dispatch::FailClosed(error),
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

/// Spawn on the blocking pool.
///
/// `openpty` plus fork and exec can take a while under memory pressure, and
/// `LaunchRequest::new` stats the working directory. On the daemon's one
/// reactor thread that window is one where nothing else is served — the same
/// cost every other call here goes out of its way to avoid.
async fn spawn_off_the_reactor(
    launch: LaunchRequest,
    geometry: PtyGeometry,
) -> Result<PendingSession, String> {
    tokio::task::spawn_blocking(move || {
        crate::runtime::spawn_session(&launch, geometry).map_err(|error| error.to_string())
    })
    .await
    .unwrap_or_else(|_| Err("the session could not be started".to_owned()))
}

/// What one `session.new` needs, once the wire has been read.
struct NewSession {
    command: Command,
    launch: LaunchRequest,
    geometry: PtyGeometry,
}

/// Start a managed session and its first Run, exactly once per command id.
///
/// The order is the one grill Q8 froze, and the obvious arrangement is the
/// broken one: claim the command id first, and only then consult the durable
/// receipt. Consulting first and claiming afterwards lets two concurrent
/// retries both read "not found" and both spawn.
async fn session_new(request: &Request, state: &Arc<DaemonState>) -> Dispatch {
    let id = request.id;
    let new = match read_session_new(request) {
        Ok(new) => new,
        Err(error) => return Dispatch::Reply(Frame::error(id, error)),
    };

    match state.commands().claim(&new.command) {
        // The same id already means something else. Nothing is executed and
        // nothing is waited for: one command id names one semantic command for
        // the life of this node's durable state.
        Claim::Conflict => Dispatch::Reply(Frame::error(id, conflict(&new.command))),
        // Another request is performing this very command. Its answer is this
        // one's answer — a second execution is the failure this whole path
        // exists to prevent.
        Claim::Waiting(concluded) => Dispatch::Reply(match in_flight::joined(concluded).await {
            Some(concluded) => answer(id, concluded),
            // Its owner ended without publishing, so nothing was completed and
            // this command may be sent again.
            None => Frame::error(
                id,
                ProtocolError::new(
                    ErrorCode::Busy,
                    "this command's first execution did not complete; send it again",
                ),
            ),
        }),
        Claim::Owner(owner) => match execute_session_new(new, state).await {
            Ok(concluded) => {
                // Published before the claim is released with `owner`, so a
                // waiter cannot slip between the two and start a second run.
                owner.publish(concluded.clone());
                Dispatch::Reply(answer(id, concluded))
            }
            // Nothing is published: the store can no longer vouch for durable
            // truth, and the daemon stops serving rather than answering from
            // it (ADR 0002, Q14).
            Err(error) => Dispatch::FailClosed(error),
        },
    }
}

/// Everything about a `session.new` that can be decided before anything runs.
fn read_session_new(request: &Request) -> Result<NewSession, ProtocolError> {
    let invalid = |detail: String| ProtocolError::new(ErrorCode::InvalidParams, detail);

    let params: SessionNewParams = match request.params.clone() {
        Some(params) => {
            serde_json::from_value(params).map_err(|source| invalid(source.to_string()))?
        }
        None => return Err(invalid("session.new needs a command".to_owned())),
    };

    let command_id = CommandId::new(params.command_id.clone())
        .map_err(|refusal| invalid(refusal.to_string()))?;

    let Some((program, arguments)) = params.argv.split_first() else {
        return Err(invalid("session.new needs a command".to_owned()));
    };

    // An absent working directory is the caller having no preference, so the
    // daemon supplies one. A directory the caller named and the daemon cannot
    // use is refused rather than quietly replaced.
    let working_directory = params
        .cwd
        .clone()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);

    let launch = LaunchRequest::new(
        program,
        arguments.iter().map(std::ffi::OsString::from),
        &working_directory,
    )
    .map_err(|refusal| invalid(refusal.to_string()))?;

    // A size from the wire is a request, not a fact: zero rows builds a page
    // list the emulator only null-checks in debug, and an unbounded one asks
    // the daemon to allocate an active area of any size at all.
    let geometry = PtyGeometry::new(params.rows.unwrap_or(24), params.cols.unwrap_or(80))
        .map_err(|impossible| invalid(impossible.to_string()))?;

    Ok(NewSession {
        command: Command::new(command_id, fingerprint(&params)),
        launch,
        geometry,
    })
}

/// The semantic identity of one `session.new`.
///
/// Every input that affects the mutation, and nothing that does not: not the
/// request id, not the connection, not when the retry was sent. A serializer
/// change or a reordered object must not split one command into two (ADR 0002,
/// Q12). An input this method later grows joins this — the rule is "all of
/// what the command means", not this particular list.
fn fingerprint(params: &SessionNewParams) -> CommandFingerprint {
    // The wire method's own name, not a second copy of it. The kind is baked
    // into the canonical fingerprint a receipt stores, so two spellings that
    // drifted apart would turn every pre-drift retry into a conflict for a
    // command that had not changed at all.
    let kind = CommandKind::new(method::SESSION_NEW).unwrap_or_else(|_| {
        // A fixed literal with no whitespace and no control characters is one
        // `CommandKind::new` accepts; this arm exists only because the type
        // refuses to assume that on a caller's behalf.
        unreachable!("{} is a usable command kind", method::SESSION_NEW)
    });
    let mut fingerprint = CommandFingerprint::builder(kind);
    // Indexed rather than joined: a separator would let one argument's content
    // impersonate a different argument list.
    for (position, argument) in params.argv.iter().enumerate() {
        fingerprint = fingerprint.input(format!("argv.{position}"), argument);
    }
    // Absence is itself an input — "the daemon chooses" is a different command
    // from "run it here" — and an absent name is how the builder records it.
    if let Some(cwd) = &params.cwd {
        fingerprint = fingerprint.input("cwd", cwd);
    }
    // Geometry is deliberately absent. It is the first attaching client's
    // presentation preference, not a property of the Session being created —
    // the daemon already treats it as optional, and the first attach reconciles
    // it anyway. It is also the one input a client cannot repeat: a terminal
    // resized between a lost response and its retry would make the retry a
    // `CommandIdConflict`, whose contract is that the id will never mean this
    // command — leaving the caller unable to retry and unable to learn what the
    // first attempt started (founder ruling, 2026-08-25).
    fingerprint.build()
}

/// Perform one `session.new`, as the owner of its command id.
///
/// `Err` is reserved for a store that can no longer vouch: everything a client
/// did wrong, and every way a runtime failed to start, is a `Concluded` its
/// waiters share.
async fn execute_session_new(
    new: NewSession,
    state: &Arc<DaemonState>,
) -> Result<Concluded, corral_state::StateError> {
    let NewSession {
        command,
        launch,
        geometry,
    } = new;

    // Consulted before anything is spawned. Spawning first and discovering
    // afterwards that this command already ran leaves two agents running, the
    // second one nobody asked for (grill Q2).
    match state.completed_managed_session(command.clone()).await {
        Ok(Some(already)) => {
            return Ok(Concluded::Accepted {
                session: already.session(),
                run: already.run(),
            });
        }
        Ok(None) => {}
        Err(error) => return refused_by_store(&command, error),
    }

    // Minted before the runtime exists, and that is the point: `RunEnded`
    // needs an id to name, and a process that exits instantly is already being
    // reaped while a store call would still be returning one. Minting an id is
    // not asserting that a runtime exists — a spawn that fails simply leaves
    // this unused (grill Q3).
    let run = corral_core::RunId::mint();

    let pending = match spawn_off_the_reactor(launch, geometry).await {
        Ok(pending) => pending,
        // The command never ran, so no Run exists to report. Saying otherwise
        // would record a runtime occurrence that never happened.
        Err(error) => {
            return Ok(Concluded::Refused {
                code: ErrorCode::InvalidParams,
                message: error,
            });
        }
    };

    // A concrete runtime occurrence now exists, so its start may be written —
    // and must be, before anything that could report its end exists. The
    // producer of `RunEnded` is created only after this commits (grill Q9).
    //
    // Two instants, because they answer different questions: when the runtime
    // began, which Corral watched, and when Corral accepted the command. ADR
    // 0002 D6 keeps them apart, and one value used for both would be the
    // conflation it exists to prevent. The first is the spawn's own, measured
    // where the process was created rather than here — the gap across a
    // blocking-pool hop and a reschedule is arbitrary under load, and an
    // instant measured after the fact is not an authoritative one.
    let began = pending.began();
    let started = match state
        .start_managed_session(
            command.clone(),
            run,
            OccurrenceTime::Authoritative(began),
            SystemTime::now(),
        )
        .await
    {
        Ok(started) => started,
        Err(error) => {
            // The child is running and its Run is not a durable fact. It is
            // hung up and reaped here rather than left alive and unlistable,
            // and no ending is reported: with no durable start there is no Run
            // to end (grill Q9).
            abandon(pending);
            return refused_by_store(&command, error);
        }
    };

    // Only a receipt this call wrote describes the runtime this call spawned.
    // A replay here would mean another execution already committed — which the
    // claim above makes impossible on one daemon, and which must still never
    // leave a second process running.
    if !started.executed() {
        abandon(pending);
        return Ok(Concluded::Accepted {
            session: started.session(),
            run: started.run(),
        });
    }

    let session = started.session();
    let handle = pending.serve(session, run, state.observations().clone());

    // The child is already running by now, so the handle must not simply be
    // dropped if the runtime registry cannot take it: the reader thread holds
    // another sender, so dropping this one would leave a live process and its
    // screen running unreachable for the daemon's lifetime.
    // Held outside the closure so a lock the daemon could not take does not
    // drop it: the closure would never run and the handle would go with it.
    let mut orphan = Some(handle);
    let stored = state.with_runtime(|runtime| {
        if let Some(handle) = orphan.take() {
            runtime.sessions.insert(handle);
        }
    });
    if stored.is_none() {
        if let Some(orphaned) = orphan.take() {
            // Its ending is still reported and still recorded: the Run is a
            // durable fact now, and a session Corral gives up on is an episode
            // that ends rather than one that stays open forever.
            orphaned.shut_down();
        }
        // Not `busy`, which invites a retry: the command has already executed
        // and its receipt is durable, so a retry would replay this same answer
        // rather than do anything different. What the caller is told is what
        // happened — a Run that started and is ending — and the session it
        // names is the one the log holds.
        error!(%session, %run, "a managed run could not be registered and was ended");
    }

    Ok(Concluded::Accepted { session, run })
}

/// End a runtime whose Run never became a durable fact.
///
/// A plain thread rather than the blocking pool, and deliberately not waited
/// on. Reaping is the one thing a child can make Corral wait for indefinitely
/// — a process may ignore a hang-up and never read its terminal — and neither
/// a client's request nor the daemon's own exit may be held by that. The
/// blocking pool would hold the exit: dropping the tokio runtime waits for
/// every blocking task that has started. This is the same shape a served
/// session's reaper already has.
fn abandon(pending: PendingSession) {
    std::thread::spawn(move || pending.abandon());
}

/// Turn a store answer into what the client is told, or fail closed.
///
/// The split is the store's own: a refusal leaves it intact and trustworthy,
/// so it is the client's to act on; a fatal state means it can no longer vouch
/// for durable truth and nothing may be answered from it (ADR 0002, Q14).
///
/// Every refusal, not a list of the ones that came to mind. A refusal this
/// arm did not name would otherwise end the daemon — and a fingerprint too
/// large for a durable row is a refusal any argv can reach, so the list was a
/// way for one client to stop every other session's control plane.
fn refused_by_store(
    command: &Command,
    error: corral_state::StateError,
) -> Result<Concluded, corral_state::StateError> {
    use corral_state::{Refusal, StateError};

    let refusal = match error {
        StateError::Refused(refusal) => refusal,
        fatal => return Err(fatal),
    };
    Ok(match refusal {
        Refusal::CommandIdConflict { .. } => Concluded::Refused {
            code: ErrorCode::CommandIdConflict,
            message: conflict(command).message,
        },
        Refusal::Busy { .. } => Concluded::Refused {
            code: ErrorCode::Busy,
            message: "the registry is held by another writer".to_owned(),
        },
        // A fixed message: the engine's own text names tables, columns and
        // paths, and a protocol error crosses the socket.
        Refusal::Constraint { .. } => Concluded::Refused {
            code: ErrorCode::InvalidParams,
            message: "the registry would not record this command".to_owned(),
        },
        // Domain text, which is written to be read by whoever sent the command.
        other => Concluded::Refused {
            code: ErrorCode::InvalidParams,
            message: other.to_string(),
        },
    })
}

fn conflict(command: &Command) -> ProtocolError {
    ProtocolError::new(
        ErrorCode::CommandIdConflict,
        format!(
            "command id {} already names a different command; nothing was executed",
            command.id().as_str()
        ),
    )
}

/// The frame one concluded command produces, for its owner and its waiters
/// alike.
fn answer(id: RequestId, concluded: Concluded) -> Frame {
    match concluded {
        Concluded::Accepted { session, run } => match serde_json::to_value(SessionNewResult {
            session_id: session.to_string(),
            run_id: run.to_string(),
        }) {
            Ok(value) => Frame::result(id, value),
            Err(source) => Frame::error(
                id,
                ProtocolError::new(ErrorCode::InvalidParams, source.to_string()),
            ),
        },
        Concluded::Refused { code, message } => Frame::error(id, ProtocolError::new(code, message)),
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
