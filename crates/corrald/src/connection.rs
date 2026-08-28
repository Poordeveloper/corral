use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use corral_core::{
    Command, CommandFingerprint, CommandId, CommandKind, CorralSessionId, NativeResumeEligibility,
    OccurrenceTime, RunEnd, RunId,
};
use corral_protocol::method::{
    self, AgentEvent, PingResult, ProviderFacts, SessionListItem, SessionListResult,
    SessionNewParams, SessionNewResult, SessionResumeParams, SessionResumeResult,
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
use crate::provider::{
    self, InjectedSettings, InjectionFailed, KnownProvider, LaunchScope, LaunchToken,
    ReportedSession,
};
use crate::runtime::{
    AttachGrant, AttachToken, LaunchRequest, ManagedSession, PendingSession, PtyGeometry,
    TerminalAccess,
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
            // A daemon originates nothing, so no response can be an answer to
            // anything this daemon sent.
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
            // Answered from this daemon's own runtime: what is live here is
            // what a list means, and the store deliberately is not unioned
            // into it (grill Q4). The registry is still asked, because a list
            // is a claim made in its name, and a store that can no longer
            // vouch for durable truth must not have one made for it.
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
        // The same gate `session.new` takes, for the same reason: a
        // continuation is a mutation, and a mutation must not be admitted
        // under the condition a read is refused.
        method::SESSION_RESUME => match state.vouch().await {
            Ok(Vouched::Yes) => session_resume(request, state).await,
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
                format!(
                    "this daemon speaks protocol {} and does not serve {other}",
                    corral_protocol::PROTOCOL_VERSION
                ),
            ),
        )),
    }
}

/// The sessions this daemon runs, in the wire's first concrete session shape.
fn session_list(id: RequestId, state: &Arc<DaemonState>) -> Frame {
    // Both under one lock. The runtime's own view of a session and what its
    // provider has reported are two halves of one row, and reading them
    // separately would let a row claim an identity for a session the other
    // half no longer holds.
    let Some(described) = state.with_runtime(|runtime| {
        runtime
            .sessions
            .describe()
            .into_iter()
            .map(|session| {
                let reported = runtime.reported.get(session.session).cloned();
                (session, reported)
            })
            .collect::<Vec<_>>()
    }) else {
        return Frame::error(
            id,
            ProtocolError::new(ErrorCode::Busy, "the runtime could not be consulted"),
        );
    };

    let sessions: Vec<serde_json::Value> = described
        .iter()
        .map(|(session, reported)| encode_session(session, reported.as_ref()))
        .collect();
    match serde_json::to_value(SessionListResult { sessions }) {
        Ok(value) => Frame::result(id, value),
        Err(source) => Frame::error(
            id,
            ProtocolError::new(ErrorCode::InvalidParams, source.to_string()),
        ),
    }
}

fn encode_session(
    session: &ManagedSession,
    reported: Option<&ReportedSession>,
) -> serde_json::Value {
    serde_json::to_value(SessionListItem {
        session_id: session.session.to_string(),
        title: session.title.clone(),
        execution_state: session.execution_state.as_str().to_owned(),
        // Always stated: this daemon knows the answer for every session it
        // runs, and absence on the wire means unknown rather than a value it
        // declined to send.
        terminal_access: Some(session.terminal_access.as_wire()),
        // Absent for a raw command, which has no provider — and absent again
        // after a restart, when live evidence is gone. Both mean unknown, and
        // neither means "there is no provider here".
        provider: reported.map(|reported| ProviderFacts {
            name: reported.provider.as_str().to_owned(),
            // A current claim: withdrawn while contested, and never a
            // conflicting id promoted into a replacement (ADR 0004 D8).
            external_id: reported
                .external_id
                .as_ref()
                .map(|id| id.as_str().to_owned()),
        }),
        agent_event: reported
            .and_then(|reported| reported.latest)
            .and_then(|fact| {
                Some(AgentEvent {
                    kind: fact.kind.as_wire(),
                    at_ms: unix_millis(fact.observed_at)?,
                })
            }),
    })
    .unwrap_or_else(|_| serde_json::json!({}))
}

/// An instant as the wire carries it, or nothing.
///
/// A clock far enough out to overflow this cannot describe an age either, and
/// a row that omits the fact says unknown — which is true — where a saturated
/// number would say something false with confidence.
fn unix_millis(at: SystemTime) -> Option<i64> {
    match at.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(since) => i64::try_from(since.as_millis()).ok(),
        Err(before) => i64::try_from(before.duration().as_millis())
            .ok()
            .map(|millis| -millis),
    }
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
    plan: LaunchPlan,
    geometry: PtyGeometry,
}

/// What the daemon is being asked to start.
///
/// The provider form cannot be turned into a `LaunchRequest` at read time: its
/// argv names a Corral-owned file that does not exist yet, and that file's
/// content names a token that has to resolve to the Session and Run this
/// request is about to mint.
enum LaunchPlan {
    /// The raw runtime harness: a command the caller composed.
    Raw(LaunchRequest),
    /// An agent Corral composes the command for, including the hook injection
    /// that makes the session attested.
    Provider {
        provider: KnownProvider,
        args: Vec<String>,
        working_directory: PathBuf,
    },
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
        Claim::Waiting(concluded) => Dispatch::Reply(
            match in_flight::joined(concluded, in_flight::JOIN_DEADLINE).await {
                Some(concluded) => answer(id, concluded, Produced::NewSession),
                // Its owner ended without publishing, or is still going after
                // longer than one may take. Either way nothing this request
                // can report was completed, and the command may be sent again
                // — which is safe because sending it again is idempotent.
                None => Frame::error(
                    id,
                    ProtocolError::new(
                        ErrorCode::Busy,
                        "this command's first execution did not complete; send it again",
                    ),
                ),
            },
        ),
        Claim::Owner(owner) => match execute_session_new(new, state).await {
            Ok(concluded) => {
                // Published before the claim is released with `owner`, so a
                // waiter cannot slip between the two and start a second run.
                owner.publish(concluded.clone());
                Dispatch::Reply(answer(id, concluded, Produced::NewSession))
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

    // An absent working directory is the caller having no preference, so the
    // daemon supplies one. A directory the caller named and the daemon cannot
    // use is refused rather than quietly replaced.
    let working_directory = params
        .cwd
        .clone()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);

    // Exactly one of the two. A request carrying both is two commands, and one
    // carrying neither is none: guessing which was meant is how a provider name
    // and a program name stop being distinct namespaces (grill Q6).
    let plan = match (&params.provider, params.argv.split_first()) {
        (Some(_), Some(_)) => {
            return Err(invalid(
                "session.new takes a provider or a command, never both".to_owned(),
            ));
        }
        (None, None) => {
            return Err(invalid(
                "session.new needs a provider or a command".to_owned(),
            ));
        }
        (Some(name), None) => {
            let provider = KnownProvider::from_name(name).ok_or_else(|| {
                ProtocolError::new(ErrorCode::UnknownProvider, unknown_provider(name))
            })?;
            // Before anything is minted or written: an argument that would
            // compete with Corral's own injection is refused rather than
            // dropped, because either silence would be a lie — one about the
            // person's configuration, the other about what Corral is watching.
            provider::refuse_arguments(provider, &params.args)
                .map_err(|refused| invalid(refused.to_string()))?;
            LaunchPlan::Provider {
                provider,
                args: params.args.clone(),
                working_directory,
            }
        }
        (None, Some((program, arguments))) => {
            // Meaningless without a provider, and refused rather than ignored:
            // silently dropping them would run a command the caller did not
            // ask for.
            if !params.args.is_empty() {
                return Err(invalid(
                    "session.new takes args only with a provider".to_owned(),
                ));
            }
            LaunchPlan::Raw(
                LaunchRequest::new(
                    program,
                    arguments.iter().map(std::ffi::OsString::from),
                    &working_directory,
                )
                .map_err(|refusal| invalid(refusal.to_string()))?,
            )
        }
    };

    // A size from the wire is a request, not a fact: zero rows builds a page
    // list the emulator only null-checks in debug, and an unbounded one asks
    // the daemon to allocate an active area of any size at all.
    let geometry = PtyGeometry::new(params.rows.unwrap_or(24), params.cols.unwrap_or(80))
        .map_err(|impossible| invalid(impossible.to_string()))?;

    Ok(NewSession {
        command: Command::new(command_id, fingerprint(&params)),
        plan,
        geometry,
    })
}

/// Name what Corral does know rather than guess what was meant.
///
/// Surface-neutral on purpose. Every client renders this string as it stands,
/// and each one's way of asking for a plain command is its own — so the
/// sentence names the agents and stops, and the surface that knows its own
/// syntax adds it (`PRODUCT.md` §8; grill Q6).
fn unknown_provider(name: &str) -> String {
    let known: Vec<&str> = KnownProvider::ALL
        .iter()
        .map(|provider| provider.as_str())
        .collect();
    format!(
        "Corral does not know how to start {name}. It knows: {}.",
        known.join(", "),
    )
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
    // The provider and its arguments are what the command means just as much as
    // an argv is: the same id carrying a different provider is a different
    // command, and a retry that changed them must conflict rather than replay.
    // Named apart from `argv` so no argument can impersonate a provider arg.
    if let Some(provider) = &params.provider {
        fingerprint = fingerprint.input("provider", provider);
    }
    for (position, argument) in params.args.iter().enumerate() {
        fingerprint = fingerprint.input(format!("args.{position}"), argument);
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
        plan,
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
    // these unused (grill Q3). The Session id is minted here for the same
    // reason plus one more: a provider launch has to embed a token naming it
    // into the process it is about to create.
    let session = CorralSessionId::mint();
    let run = corral_core::RunId::mint();

    let (launch, injected) = match plan {
        LaunchPlan::Raw(launch) => (launch, None),
        LaunchPlan::Provider {
            provider,
            args,
            working_directory,
        } => {
            match compose_provider_launch(
                state,
                session,
                run,
                provider,
                SessionOwnership::CreatedHere,
                &working_directory,
                |settings| provider::launch_argv(provider, settings, &args),
            )
            .await
            {
                Ok(composed) => composed,
                // Nothing has been spawned, so nothing is left running. A managed
                // session that could not be given its hook injection is refused
                // rather than started unattested: a session that looks managed and
                // can never report is worse than one that did not start.
                Err(message) => {
                    return Ok(Concluded::Refused {
                        code: ErrorCode::InvalidParams,
                        message,
                    });
                }
            }
        }
    };

    let pending = match spawn_off_the_reactor(launch, geometry).await {
        Ok(pending) => pending,
        // The command never ran, so no Run exists to report. Saying otherwise
        // would record a runtime occurrence that never happened.
        Err(error) => {
            abandon_injection(state, injected);
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
            session,
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
            abandon_injection(state, injected);
            return refused_by_store(&command, error);
        }
    };

    // Only a receipt this call wrote describes the runtime this call spawned.
    // A replay here would mean another execution already committed — which the
    // claim above makes impossible on one daemon, and which must still never
    // leave a second process running.
    if !started.executed() {
        abandon(pending);
        abandon_injection(state, injected);
        return Ok(Concluded::Accepted {
            session: started.session(),
            run: started.run(),
        });
    }

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

/// What one `session.resume` needs, once the wire has been read.
struct ResumeSession {
    command: Command,
    session: CorralSessionId,
}

/// Continue an existing Session's provider session as a new Run, exactly once
/// per command id.
///
/// The same claim-then-consult order `session.new` uses, for the same reason:
/// two concurrent retries that both read "no receipt" would both start a Run.
async fn session_resume(request: &Request, state: &Arc<DaemonState>) -> Dispatch {
    let id = request.id;
    let resume = match read_session_resume(request) {
        Ok(resume) => resume,
        Err(error) => return Dispatch::Reply(Frame::error(id, error)),
    };

    match state.commands().claim(&resume.command) {
        Claim::Conflict => Dispatch::Reply(Frame::error(id, conflict(&resume.command))),
        Claim::Waiting(concluded) => Dispatch::Reply(
            match in_flight::joined(concluded, in_flight::JOIN_DEADLINE).await {
                Some(concluded) => answer(id, concluded, Produced::ResumedSession),
                None => Frame::error(
                    id,
                    ProtocolError::new(
                        ErrorCode::Busy,
                        "this command's first execution did not complete; send it again",
                    ),
                ),
            },
        ),
        Claim::Owner(owner) => match execute_session_resume(resume, state).await {
            Ok(concluded) => {
                owner.publish(concluded.clone());
                Dispatch::Reply(answer(id, concluded, Produced::ResumedSession))
            }
            Err(error) => Dispatch::FailClosed(error),
        },
    }
}

fn read_session_resume(request: &Request) -> Result<ResumeSession, ProtocolError> {
    let invalid = |detail: String| ProtocolError::new(ErrorCode::InvalidParams, detail);

    let params: SessionResumeParams = match request.params.clone() {
        Some(params) => {
            serde_json::from_value(params).map_err(|source| invalid(source.to_string()))?
        }
        None => return Err(invalid("session.resume needs a session".to_owned())),
    };
    let command_id = CommandId::new(params.command_id.clone())
        .map_err(|refusal| invalid(refusal.to_string()))?;
    let session: CorralSessionId = params
        .session_id
        .parse()
        .map_err(|error: corral_core::MalformedId| invalid(error.to_string()))?;

    let kind = CommandKind::new(method::SESSION_RESUME)
        .unwrap_or_else(|_| unreachable!("{} is a usable command kind", method::SESSION_RESUME));
    // The Session is the whole of what this command means. There is no argv to
    // fingerprint — Corral composes it from what it recorded — so a retry
    // naming a different Session is a different command, and that is the only
    // way this one can differ.
    let fingerprint = CommandFingerprint::builder(kind)
        .input("session", session.to_string())
        .build();

    Ok(ResumeSession {
        command: Command::new(command_id, fingerprint),
        session,
    })
}

/// Why a Session cannot be continued right now.
///
/// Every arm is a fact stated to the person, and none of them has an override.
/// M1 offers no `--force`, no "I know it is dead", and no pid heuristic: a
/// second native resume of a provider session that may still be live is two
/// executions driving one conversation (grill Q7).
enum ResumeRefused {
    /// The Session exists and is eligible, but this daemon did not launch it
    /// and so does not know where it ran.
    ///
    /// A known boundary rather than an oversight: where a Run ran is live
    /// state on its handle, and a daemon holding no client and no live Run
    /// exits after its idle grace — so a continuation outlives the provider
    /// process but not the daemon. The plan's "Known limitation" section names
    /// what repairing it needs.
    NotThisDaemon,
    /// The live runtime could not be consulted at all. Not a fact about this
    /// Session — the same request may simply be sent again.
    RuntimeUnavailable,
    IdentityUnknown,
    Eligibility(NativeResumeEligibility),
    UnknownProvider(String),
    RunStillLive,
    EndUnverifiable,
    NoPreviousRun,
    /// Which Run was the most recent episode cannot be established, so which
    /// ending governs cannot be either.
    EpisodeOrderUnknown,
}

/// Decide whether a continuation may happen, before anything is spawned.
///
/// Identity first, then the runtime preconditions. The order is the one the
/// design states, and it matters: a contested Session that has also just been
/// restarted must be refused for the reason that will not go away, not for the
/// one that would.
async fn resume_plan(
    state: &Arc<DaemonState>,
    session: CorralSessionId,
) -> Result<Result<ResumePlan, ResumeRefused>, corral_state::StateError> {
    let Some(binding) = state.provider_session_binding(session).await? else {
        return Ok(Err(ResumeRefused::IdentityUnknown));
    };
    match binding.native_resume_eligibility() {
        NativeResumeEligibility::Eligible => {}
        refused => return Ok(Err(ResumeRefused::Eligibility(refused))),
    }
    let Some(provider) = KnownProvider::from_name(binding.key().provider().as_str()) else {
        return Ok(Err(ResumeRefused::UnknownProvider(
            binding.key().provider().as_str().to_owned(),
        )));
    };

    let runs = state.runs_of(session).await?;
    if runs.iter().any(|run| run.end().is_none()) {
        return Ok(Err(ResumeRefused::RunStillLive));
    }
    // `runs_of` parks a Run whose start the runtime could not state at the end
    // of the list, so its last entry is the most recent episode only while
    // every start is authoritative. That holds of every Run a launch creates
    // and is not something a control decision may assume of a Run some later
    // phase records; reading the wrong episode's end is what would turn an
    // unverifiable ending into a resumable one.
    if runs
        .iter()
        .any(|run| run.started_at().authoritative().is_none())
    {
        return Ok(Err(ResumeRefused::EpisodeOrderUnknown));
    }
    match runs.last().map(corral_core::Run::end) {
        None => return Ok(Err(ResumeRefused::NoPreviousRun)),
        Some(Some(RunEnd::Exited(_))) => {}
        // Unreachable given the live check above, and stated rather than
        // wildcarded so a new end state has to be decided rather than
        // defaulted.
        Some(None) => return Ok(Err(ResumeRefused::RunStillLive)),
        Some(Some(RunEnd::Unverifiable)) => return Ok(Err(ResumeRefused::EndUnverifiable)),
    }

    // Live state, and the last precondition on purpose: a daemon that did not
    // launch this Session does not know where it ran, and a provider resolves
    // which of its own sessions an id names by the directory it was started
    // in. Substituting one would ask for a conversation that is not there.
    // The two `None`s here are different answers and are kept apart. A runtime
    // that could not be consulted is a lock a holder panicked under, which says
    // nothing about this Session; a Session the runtime does not hold is the
    // factual claim below.
    let Some(known) = state.with_runtime(|runtime| {
        runtime
            .sessions
            .get(session)
            .map(|handle| handle.working_directory().to_path_buf())
    }) else {
        return Ok(Err(ResumeRefused::RuntimeUnavailable));
    };
    let Some(working_directory) = known else {
        return Ok(Err(ResumeRefused::NotThisDaemon));
    };

    Ok(Ok(ResumePlan {
        provider,
        external_id: binding.key().external_id().clone(),
        working_directory,
    }))
}

/// Everything a continuation needs, once it is allowed to happen.
struct ResumePlan {
    provider: KnownProvider,
    external_id: corral_core::ExternalId,
    working_directory: PathBuf,
}

/// Perform one `session.resume`, as the owner of its command id.
async fn execute_session_resume(
    resume: ResumeSession,
    state: &Arc<DaemonState>,
) -> Result<Concluded, corral_state::StateError> {
    let ResumeSession { command, session } = resume;

    // Before the receipt is consulted, and held past the commit. Two different
    // command ids continuing one Session would otherwise both find nothing
    // running and both spawn, putting two provider processes on one
    // conversation — which the version matrix records a provider will happily
    // allow (grill Q7).
    let Some(_continuing) = state.claim_continuation(session) else {
        return Ok(Concluded::Refused {
            code: ErrorCode::Busy,
            message: "Corral is already continuing this session".to_owned(),
        });
    };

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

    let plan = match resume_plan(state, session).await {
        Ok(Ok(plan)) => plan,
        Ok(Err(refused)) => {
            return Ok(Concluded::Refused {
                // A refusal a caller may simply send again is `busy`; every
                // other one is a state this request cannot change by repeating.
                code: match refused {
                    ResumeRefused::RuntimeUnavailable => ErrorCode::Busy,
                    _ => ErrorCode::InvalidParams,
                },
                message: refused.to_string(),
            });
        }
        Err(error) => return refused_by_store(&command, error),
    };

    let run = RunId::mint();
    let (launch, injected) = match compose_provider_launch(
        state,
        session,
        run,
        plan.provider,
        SessionOwnership::Preexisting,
        &plan.working_directory,
        |settings| provider::resume_argv(plan.provider, &plan.external_id, settings),
    )
    .await
    {
        Ok(composed) => composed,
        Err(message) => {
            return Ok(Concluded::Refused {
                code: ErrorCode::InvalidParams,
                message,
            });
        }
    };

    // The geometry a resumed Run is born at. The request carries none — a
    // continuation is asked for by a person looking at a list, not by a
    // terminal offering its size — and the first attach reconciles it, exactly
    // as it does for a session started without one.
    let geometry = PtyGeometry::new(24, 80).unwrap_or_else(|impossible| {
        unreachable!("a 24x80 terminal is a usable geometry: {impossible}")
    });
    let pending = match spawn_off_the_reactor(launch, geometry).await {
        Ok(pending) => pending,
        Err(error) => {
            abandon_injection(state, injected);
            return Ok(Concluded::Refused {
                code: ErrorCode::InvalidParams,
                message: error,
            });
        }
    };

    let began = pending.began();
    let started = match state
        .resume_managed_session(
            command.clone(),
            session,
            run,
            OccurrenceTime::Authoritative(began),
            SystemTime::now(),
        )
        .await
    {
        Ok(started) => started,
        Err(error) => {
            abandon(pending);
            abandon_injection(state, injected);
            return refused_by_store(&command, error);
        }
    };

    if !started.executed() {
        abandon(pending);
        abandon_injection(state, injected);
        return Ok(Concluded::Accepted {
            session: started.session(),
            run: started.run(),
        });
    }

    let handle = pending.serve(session, run, state.observations().clone());
    // The prior Run's final screen is superseded by the new Run's live screen:
    // one Session shows one runtime, and the record it replaces is the episode
    // that ended (ADR 0007 L1).
    let mut orphan = Some(handle);
    let stored = state.with_runtime(|runtime| {
        if let Some(handle) = orphan.take() {
            runtime.sessions.insert(handle);
        }
    });
    if stored.is_none() {
        if let Some(orphaned) = orphan.take() {
            orphaned.shut_down();
        }
        error!(%session, %run, "a resumed run could not be registered and was ended");
    }

    Ok(Concluded::Accepted { session, run })
}

impl std::fmt::Display for ResumeRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotThisDaemon => f.write_str(
                "this session was not started by the running Corral daemon, so Corral does not \
                 know where it ran and will not continue it somewhere else",
            ),
            Self::RuntimeUnavailable => {
                f.write_str("Corral could not check this session just now; try again")
            }
            Self::IdentityUnknown => f.write_str(
                "Corral has not learned which provider session this is, so there is nothing to \
                 continue",
            ),
            Self::Eligibility(NativeResumeEligibility::IdentityContested) => f.write_str(
                "this session reported a provider identity that contradicts the one Corral \
                 accepted, so Corral will not continue it",
            ),
            Self::Eligibility(NativeResumeEligibility::AssuranceTooWeak) => f.write_str(
                "Corral is not sure enough which provider session this is to continue it",
            ),
            Self::Eligibility(NativeResumeEligibility::Eligible) => {
                f.write_str("this session can be continued")
            }
            Self::UnknownProvider(name) => write!(
                f,
                "this session belongs to {name}, which this build does not know how to continue"
            ),
            Self::RunStillLive => {
                f.write_str("this session is still running, so there is nothing to continue")
            }
            Self::EndUnverifiable => f.write_str(
                "Corral cannot verify that the previous run has exited, so it will not resume \
                 this provider session automatically",
            ),
            Self::NoPreviousRun => {
                f.write_str("Corral has no record of this session ever having started")
            }
            Self::EpisodeOrderUnknown => f.write_str(
                "Corral cannot establish which run of this session was the most recent, so it \
                 will not resume this provider session automatically",
            ),
        }
    }
}

/// Build the launch of a managed provider session, hook injection included.
///
/// The order is load-bearing. The token is minted and registered *before* the
/// process exists, because a child fires its first hook within milliseconds of
/// starting and a token that became resolvable afterwards would lose the very
/// event identity is learned from. Then the Corral-owned settings file, then
/// the argv that names it.
///
/// One function for both a fresh launch and a continuation, because the two
/// differ in exactly one thing — the arguments — and everything the order
/// above protects is the same for both. `argv` receives the injected file's
/// path and returns the provider's command line.
///
/// The file is written on the blocking pool. Locating the relay stats a path
/// and publishing the settings ends in an `fsync`, which on a loaded or
/// network-backed filesystem is tens to hundreds of milliseconds — and this
/// daemon has one reactor thread. Spending it here would stop every other
/// request, stop the hook endpoint accepting, and push relays past their
/// interference budget, which is the same reason the spawn and every store
/// call are already moved off it.
async fn compose_provider_launch(
    state: &Arc<DaemonState>,
    session: CorralSessionId,
    run: RunId,
    provider: KnownProvider,
    ownership: SessionOwnership,
    working_directory: &std::path::Path,
    argv: impl FnOnce(&std::path::Path) -> Vec<std::ffi::OsString>,
) -> Result<(LaunchRequest, Option<Injected>), String> {
    let scope = LaunchScope {
        session,
        run,
        provider,
    };
    let token = state
        .mint_launch_token(scope)
        .map_err(|_| InjectionFailed::NoRandomness.to_string())?;
    // Recorded with the token, so the first fact to arrive is attributed to
    // the agent Corral started rather than to whatever a payload claims.
    state.with_runtime(|runtime| runtime.reported.launched(session, provider));
    // Nothing below leaves a half-made launch behind: a token that named a
    // process nobody started would keep resolving for the daemon's whole life.
    //
    // The Session is dropped only when this launch is what brought it into
    // being. A continuation names a Session that already exists and already
    // has evidence — its provider, its identity, the last fact it reported —
    // and forgetting that would blank a live row over a launch that failed.
    let forget = move |state: &Arc<DaemonState>| {
        state.forget_launch_token(token);
        if ownership == SessionOwnership::CreatedHere {
            state.with_runtime(|runtime| runtime.reported.forget(session));
        }
    };

    let launch_dir = state.launch_dir().to_path_buf();
    let written = tokio::task::spawn_blocking(move || {
        provider::launch::relay_command(provider, token)
            .and_then(|relay| InjectedSettings::write(&launch_dir, run, provider, &relay))
            .map_err(|failed| failed.to_string())
    })
    .await
    .unwrap_or_else(|_| Err("the launch could not be prepared".to_owned()));
    let settings = match written {
        Ok(settings) => settings,
        Err(failed) => {
            forget(state);
            return Err(failed);
        }
    };

    match LaunchRequest::new(
        provider::program(provider),
        argv(settings.path()),
        working_directory,
    ) {
        Ok(launch) => Ok((
            launch,
            Some(Injected {
                token,
                session,
                run,
                ownership,
            }),
        )),
        Err(refusal) => {
            InjectedSettings::remove_for(state.launch_dir(), run);
            forget(state);
            Err(refusal.to_string())
        }
    }
}

/// Whether a launch is what brought its Session into being.
///
/// The two paths differ in exactly one consequence — what may be undone when
/// the launch fails — and a boolean at the call site would not have said which
/// way round it read.
///
/// Deliberately not called an origin: `provider::SessionOrigin` is the
/// normalized answer to how a *provider* session started, and one crate may
/// not spell two unrelated concepts the same way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SessionOwnership {
    /// `session.new`: the Session id was minted for this launch and names
    /// nothing yet.
    CreatedHere,
    /// `session.resume`: the Session already exists and already has evidence.
    Preexisting,
}

/// What a launch that may still be abandoned left behind.
struct Injected {
    token: LaunchToken,
    session: CorralSessionId,
    /// The Run whose file this is. Carried rather than passed beside it: the
    /// undo below deletes a live launch's settings file if the two ever
    /// disagree, and a caller that cannot name the wrong Run cannot make that
    /// mistake.
    run: RunId,
    ownership: SessionOwnership,
}

/// Undo a launch that never became one.
///
/// The file is removed here rather than left for the startup sweep, because
/// this is the moment its owner is known not to exist. The token goes with it:
/// it names a Session and Run nothing can ever present.
fn abandon_injection(state: &Arc<DaemonState>, injected: Option<Injected>) {
    let Some(injected) = injected else {
        return;
    };
    InjectedSettings::remove_for(state.launch_dir(), injected.run);
    state.forget_launch_token(injected.token);
    if injected.ownership == SessionOwnership::CreatedHere {
        state.with_runtime(|runtime| runtime.reported.forget(injected.session));
    }
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

/// Which method's result shape an accepted outcome is encoded as.
///
/// The two carry the same two identities today and are still separate wire
/// types, so encoding one as the other would make a later divergence look
/// safe.
#[derive(Clone, Copy)]
enum Produced {
    NewSession,
    ResumedSession,
}

/// The frame one concluded command produces, for its owner and its waiters
/// alike.
fn answer(id: RequestId, concluded: Concluded, produced: Produced) -> Frame {
    match concluded {
        Concluded::Accepted { session, run } => match match produced {
            Produced::NewSession => serde_json::to_value(SessionNewResult {
                session_id: session.to_string(),
                run_id: run.to_string(),
            }),
            Produced::ResumedSession => serde_json::to_value(SessionResumeResult {
                session_id: session.to_string(),
                run_id: run.to_string(),
            }),
        } {
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
        // What this daemon publishes it also enforces. `session.list` says
        // whether a session's terminal can be served, and granting one it has
        // just called unavailable would leave a client — including every one
        // that cannot read the field, which the wire contract tells to try and
        // report whatever comes back — holding a channel whose only possible
        // content is an error, instead of a refusal that names the reason.
        if handle.terminal_access() == TerminalAccess::Unavailable {
            return None;
        }
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
            format!(
                "{} takes no parameters in protocol {}",
                request.method,
                corral_protocol::PROTOCOL_VERSION
            ),
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
