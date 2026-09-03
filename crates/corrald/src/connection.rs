use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use corral_core::{
    Command, CommandFingerprint, CommandId, CommandKind, CorralSessionId, ProviderId, RunId,
};

use corral_protocol::method::{
    self, AgentEvent, IntegrationParams, IntegrationResult, PingResult, ProviderFacts,
    SessionListItem, SessionListResult, SessionNewParams, SessionNewResult, SessionResumeParams,
    SessionResumeResult, TerminalAttachParams, TerminalAttachResult,
};
use corral_protocol::{
    ClientHello, Compatibility, ConnectionRole, ErrorCode, Frame, FrameError, FrameReader,
    FrameWriter, ProtocolError, Request, RequestId, ServerHello, capability, compatible,
    local_versions,
};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::watch;
use tracing::{debug, error, warn};

use crate::in_flight::{self, Claim, Concluded};
use crate::integration;
use crate::lifecycle::{EstablishedGuard, Lifecycle, ShutdownReason};
use crate::managed_launch::{
    self, Committed, ResumeRefused, SessionOwnership, compose_provider_launch, resume_plan,
};
use crate::policy::DaemonPolicy;
use crate::provider::launch::RelayInvocation;
use crate::provider::{self, KnownProvider, ReportedSession};
use crate::runtime::{
    AttachGrant, AttachToken, LaunchRequest, ManagedSession, PtyGeometry, TerminalAccess,
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
        // Advertised rather than left to be discovered by a request that
        // fails: `session.resume` and a provider-named `session.new` are
        // additive, so the version says nothing about them, and a client that
        // could not ask would offer a person Continue against a daemon too old
        // to serve it and report `method_not_found` as if the person had asked
        // for something wrong.
        //
        // And withheld by the same reasoning when this daemon could not bind
        // its hook endpoint, because then it will refuse every managed launch
        // for the life of the process. Serving the method is not the question
        // a capability answers; whether the offer leads anywhere is. The bind
        // is attempted before the first hello, so the answer is known here.
        capabilities: {
            // Unconditional: every session this daemon can see is derived
            // over, whether or not it can start one, and a client that must
            // decide whether to show a state at all asks here rather than
            // discovering `method_not_found` in front of a person.
            let mut capabilities = BTreeSet::from([capability::ATTENTION.to_owned()]);
            if state.hook_endpoint_was_bound() {
                capabilities.insert(capability::MANAGED_SESSIONS.to_owned());
            }
            capabilities
        },
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
        // Live state only: nothing durable is read or written, so neither
        // takes the registry gate. The summary is the daemon's projection of
        // its current items; an acknowledgement names the item it saw.
        method::ATTENTION_SUMMARY => match no_params(request) {
            Ok(()) => Dispatch::Reply(attention_summary(id, state)),
            Err(error) => Dispatch::Reply(Frame::error(id, error)),
        },
        method::ATTENTION_ACKNOWLEDGE => Dispatch::Reply(attention_acknowledge(request, state)),
        method::ATTENTION_REPORT => Dispatch::Reply(attention_report(request, state).await),
        method::ATTENTION_DISPUTE => Dispatch::Reply(attention_dispute(request, state).await),
        // Status reads a file rather than the registry, but it is still a
        // claim made in the daemon's name and the two mutations write durable
        // intent, so all three take the same gate as any other request.
        method::INTEGRATION_STATUS | method::INTEGRATION_ENABLE | method::INTEGRATION_DISABLE => {
            match state.vouch().await {
                Ok(Vouched::Yes) => integration_request(request, state).await,
                Ok(Vouched::NotNow) => Dispatch::Reply(Frame::error(
                    id,
                    ProtocolError::new(ErrorCode::Busy, "the registry is held by another writer"),
                )),
                Err(error) => Dispatch::FailClosed(error),
            }
        }
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
    //
    // The runtimes outside Corral are read from the sweep first and judged
    // under the same lock, because a row the daemon derived attention for and
    // the count that includes it are one fact seen twice: a heading that says
    // one session needs you, over a row that says nothing, is the daemon
    // contradicting itself.
    let external = state.seen_runtimes().snapshot();
    let Some((described, externally)) = state.with_runtime(|runtime| {
        let described = runtime
            .sessions
            .describe()
            .into_iter()
            .map(|session| {
                let reported = runtime.reported.get(session.session).cloned();
                let attention = attention_facts(&runtime.attention, session.session);
                (session, reported, attention)
            })
            .collect::<Vec<_>>();
        let externally: Vec<Option<method::AttentionFacts>> = external
            .iter()
            .map(|candidate| {
                candidate
                    .identified()
                    .and_then(|identified| attention_facts(&runtime.attention, identified.session))
            })
            .collect();
        (described, externally)
    }) else {
        return Frame::error(
            id,
            ProtocolError::new(ErrorCode::Busy, "the runtime could not be consulted"),
        );
    };

    let mut sessions: Vec<serde_json::Value> = described
        .into_iter()
        .map(|(session, reported, attention)| {
            encode_session(&session, reported.as_ref(), attention)
        })
        .collect();
    // After the managed rows: the runtimes outside Corral. A session Corral
    // started reports through its own channel; these are the ones the
    // process table showed or a token-less delivery corroborated, which is
    // exactly the session a person most needs reminding of (ADR 0014 D2).
    sessions.extend(
        external
            .iter()
            .zip(externally)
            .map(|(candidate, attention)| encode_external(candidate, attention)),
    );
    match serde_json::to_value(SessionListResult { sessions }) {
        Ok(value) => Frame::result(id, value),
        Err(source) => Frame::error(
            id,
            ProtocolError::new(ErrorCode::InvalidParams, source.to_string()),
        ),
    }
}

fn attention_summary(id: RequestId, state: &Arc<DaemonState>) -> Frame {
    // A runtime nobody can read is not a runtime with nothing to say. Zeroes
    // here would be Corral telling a person that nothing needs them on the
    // strength of state nobody finished writing.
    let Some(summary) = state.with_runtime(|runtime| runtime.attention.summary()) else {
        return Frame::error(
            id,
            ProtocolError::new(ErrorCode::Busy, "the runtime could not be consulted"),
        );
    };
    match serde_json::to_value(summary) {
        Ok(value) => Frame::result(id, value),
        Err(source) => Frame::error(
            id,
            ProtocolError::new(ErrorCode::InvalidParams, source.to_string()),
        ),
    }
}

fn attention_acknowledge(request: &Request, state: &Arc<DaemonState>) -> Frame {
    let id = request.id;
    let params: method::AttentionAcknowledgeParams = match request
        .params
        .clone()
        .map(serde_json::from_value)
        .transpose()
    {
        Ok(Some(params)) => params,
        Ok(None) => {
            return Frame::error(
                id,
                ProtocolError::new(
                    ErrorCode::InvalidParams,
                    "attention.acknowledge needs params",
                ),
            );
        }
        Err(source) => {
            return Frame::error(
                id,
                ProtocolError::new(ErrorCode::InvalidParams, source.to_string()),
            );
        }
    };
    let (session, item) = match (
        params.session_id.parse::<CorralSessionId>(),
        params
            .attention_item_id
            .parse::<corral_core::AttentionItemId>(),
    ) {
        (Ok(session), Ok(item)) => (session, item),
        (Err(source), _) | (_, Err(source)) => {
            return Frame::error(
                id,
                ProtocolError::new(ErrorCode::InvalidParams, source.to_string()),
            );
        }
    };
    let outcome = state.with_runtime(|runtime| runtime.attention.acknowledge(session, item));
    match outcome {
        Some(crate::attention::Acknowledgement::Acknowledged) => {
            Frame::result(id, serde_json::json!({}))
        }
        // Both mean the same thing to a client: reload the row. The item that
        // replaced a stale one stays unacknowledged either way (grill Q18).
        Some(crate::attention::Acknowledgement::StaleAttentionItem) => Frame::error(
            id,
            ProtocolError::new(
                ErrorCode::StaleAttentionItem,
                "that attention item is no longer the session's current one",
            ),
        ),
        Some(crate::attention::Acknowledgement::NoCurrentItem) => Frame::error(
            id,
            ProtocolError::new(
                ErrorCode::StaleAttentionItem,
                "the session has no current attention item",
            ),
        ),
        None => Frame::error(
            id,
            ProtocolError::new(ErrorCode::Busy, "the runtime could not be consulted"),
        ),
    }
}

/// The journal read back per day. Diagnostics: a daemon without a journal
/// answers an empty report, which is the truth about what it can report.
async fn attention_report(request: &Request, state: &Arc<DaemonState>) -> Frame {
    let id = request.id;
    let params: method::AttentionReportParams = match request
        .params
        .clone()
        .map(serde_json::from_value)
        .transpose()
    {
        Ok(params) => params.unwrap_or_default(),
        Err(source) => {
            return Frame::error(
                id,
                ProtocolError::new(ErrorCode::InvalidParams, source.to_string()),
            );
        }
    };
    // The filter compares `since` against the journal's own day names, so a
    // value in another shape would answer a report that looks valid and
    // means nothing.
    if let Some(since) = params.since.as_deref()
        && !crate::attention::names_a_day(since)
    {
        return Frame::error(
            id,
            ProtocolError::new(
                ErrorCode::InvalidParams,
                "attention.report since must be a day as YYYY-MM-DD",
            ),
        );
    }
    let Some(dir) = state.journal_dir() else {
        return Frame::result(
            id,
            serde_json::to_value(method::AttentionReportResult::default()).unwrap_or_default(),
        );
    };
    // A reader that failed is not a journal with nothing in it. Answering
    // zero days would present incomplete evidence as a quiet period, which
    // is the one thing D8 says the journal must never do silently.
    let report = match tokio::task::spawn_blocking(move || crate::attention::report(&dir)).await {
        Ok(Ok(report)) => report,
        Ok(Err(source)) => {
            return Frame::error(id, ProtocolError::new(ErrorCode::Busy, source.to_string()));
        }
        Err(_) => {
            return Frame::error(
                id,
                ProtocolError::new(ErrorCode::Busy, "the attention journal could not be read"),
            );
        }
    };
    let days = report
        .days
        .into_iter()
        .filter(|day| {
            params
                .since
                .as_ref()
                .is_none_or(|since| day.date.as_str() >= since.as_str())
        })
        .map(|day| method::AttentionDayFacts {
            date: day.date,
            transitions: day.transitions,
            into_needs_you: day.into_needs_you,
            into_ready: day.into_ready,
            disputes: day.disputes,
            incomplete: day.incomplete,
        })
        .collect();
    match serde_json::to_value(method::AttentionReportResult { days }) {
        Ok(value) => Frame::result(id, value),
        Err(source) => Frame::error(
            id,
            ProtocolError::new(ErrorCode::InvalidParams, source.to_string()),
        ),
    }
}

/// A person says the current item was wrong. Recorded against the item they
/// named, and stale when that item is no longer current — a dispute of the
/// one that just resolved is never attributed to its replacement
/// (grill Q34).
async fn attention_dispute(request: &Request, state: &Arc<DaemonState>) -> Frame {
    let id = request.id;
    let params: method::AttentionDisputeParams = match request
        .params
        .clone()
        .map(serde_json::from_value)
        .transpose()
    {
        Ok(Some(params)) => params,
        Ok(None) => {
            return Frame::error(
                id,
                ProtocolError::new(ErrorCode::InvalidParams, "attention.dispute needs params"),
            );
        }
        Err(source) => {
            return Frame::error(
                id,
                ProtocolError::new(ErrorCode::InvalidParams, source.to_string()),
            );
        }
    };
    let session = match params.session_id.parse::<CorralSessionId>() {
        Ok(session) => session,
        Err(source) => {
            return Frame::error(
                id,
                ProtocolError::new(ErrorCode::InvalidParams, source.to_string()),
            );
        }
    };
    let named = match params
        .attention_item_id
        .as_deref()
        .map(str::parse::<corral_core::AttentionItemId>)
        .transpose()
    {
        Ok(named) => named,
        Err(source) => {
            return Frame::error(
                id,
                ProtocolError::new(ErrorCode::InvalidParams, source.to_string()),
            );
        }
    };
    // Without the ledger there is no current item to compare the dispute
    // against, and recording one anyway would enter "not stale" as a fact
    // about an item nobody looked at.
    let Some((current, claims)) = state.with_runtime(|runtime| {
        (
            runtime.attention.state(session).and_then(|(_, item)| item),
            runtime.attention.claims(session),
        )
    }) else {
        return Frame::error(
            id,
            ProtocolError::new(ErrorCode::Busy, "the runtime could not be consulted"),
        );
    };
    let current = current.map(|item| item.id());
    // What the disputed state rested on, for the person triaging the dispute
    // later: the journal record names the item, the log names the evidence.
    tracing::debug!(%session, ?claims, "an attention item was disputed");
    let (item, stale) = match (named, current) {
        (Some(named), Some(current)) => (Some(named), named != current),
        (Some(named), None) => (Some(named), true),
        (None, current) => (current, false),
    };
    let recording = Arc::clone(state);
    let now = std::time::SystemTime::now();
    let _ = tokio::task::spawn_blocking(move || {
        recording.journal_append(
            now,
            vec![crate::attention::Record::Dispute(
                crate::attention::DisputeRecord {
                    session,
                    item,
                    stale,
                },
            )],
        );
    })
    .await;
    match serde_json::to_value(method::AttentionDisputeResult { stale }) {
        Ok(value) => Frame::result(id, value),
        Err(source) => Frame::error(
            id,
            ProtocolError::new(ErrorCode::InvalidParams, source.to_string()),
        ),
    }
}

/// The attention projection for one row, as the ledger holds it.
fn attention_facts(
    attention: &crate::attention::Ledger,
    session: CorralSessionId,
) -> Option<method::AttentionFacts> {
    let (state, item) = attention.state(session)?;
    let wire_state = |main: corral_core::MainState| match main {
        corral_core::MainState::Working => method::AttentionWireState::Working,
        corral_core::MainState::NeedsYou => method::AttentionWireState::NeedsYou,
        corral_core::MainState::Ready => method::AttentionWireState::Ready,
        corral_core::MainState::Unknown => method::AttentionWireState::Unknown,
        corral_core::MainState::Exited => method::AttentionWireState::Exited,
    };
    let unix_ms = |at: std::time::SystemTime| {
        at.duration_since(std::time::SystemTime::UNIX_EPOCH)
            .ok()
            .and_then(|d| i64::try_from(d.as_millis()).ok())
            .unwrap_or(0)
    };
    Some(method::AttentionFacts {
        state: wire_state(state.main()),
        since_unix_ms: unix_ms(state.since()),
        last_known: state.last_known().map(|known| method::LastKnownFacts {
            state: wire_state(known.state()),
            at_unix_ms: unix_ms(known.at()),
        }),
        items: item
            .map(|item| method::AttentionItemFacts {
                attention_item_id: item.id().to_string(),
                reason: match item.reason() {
                    corral_core::AttentionReason::NeedsInput => {
                        method::AttentionReasonWire::NeedsInput
                    }
                    corral_core::AttentionReason::TurnComplete => {
                        method::AttentionReasonWire::TurnComplete
                    }
                    corral_core::AttentionReason::RuntimeEnded => {
                        method::AttentionReasonWire::Unrecognized("runtime_ended".to_owned())
                    }
                },
                since_unix_ms: unix_ms(item.since()),
                acknowledged: item.acknowledged(),
            })
            .into_iter()
            .collect(),
    })
}

fn encode_session(
    session: &ManagedSession,
    reported: Option<&ReportedSession>,
    attention: Option<method::AttentionFacts>,
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
            .and_then(|fact| AgentEvent::at(fact.kind.as_wire(), fact.observed_at)),
        // Known by construction: this daemon started it.
        origin: Some(method::ORIGIN_MANAGED.to_owned()),
        location_hint: None,
        attention,
    })
    .unwrap_or_else(|_| serde_json::json!({}))
}

/// One provider runtime outside Corral, as a row.
///
/// The row claims exactly what its evidence supports and no more. Runtime
/// recognition alone says a supported provider runtime appears to be running
/// here, and its execution state is the only thing the process table can
/// speak to; such a row carries no provider identity, because the table
/// holds none (grill Q5, Q6′). Identity arrives on the delivery path, and a
/// runtime discovery has identified is shown under the Session it was found
/// to be carrying, with that identity — the same row, no longer provisional.
///
/// Until then its `session_id` is the Corral identity minted for this
/// runtime's incarnation, so the row is stable across passes without the pid
/// ever becoming an identity.
fn encode_external(
    candidate: &crate::sweep::RuntimeCandidate,
    attention: Option<method::AttentionFacts>,
) -> serde_json::Value {
    let identified = candidate.identified();
    serde_json::to_value(SessionListItem {
        session_id: identified
            .map_or(candidate.provisional_id(), |identified| identified.session)
            .to_string(),
        title: candidate.provider().as_str().to_owned(),
        // The process is there; nothing else about it is known.
        execution_state: "running".to_owned(),
        // Corral owns no terminal for a process it did not start, and the
        // refusal is honest rather than a capability it might grow later.
        terminal_access: Some(corral_protocol::method::TerminalAccess::Unavailable),
        provider: Some(ProviderFacts {
            name: candidate.provider().as_str().to_owned(),
            // Absent is unknown, which is exactly right for a runtime no
            // delivery has named yet.
            external_id: identified.map(|identified| identified.external_id.as_str().to_owned()),
        }),
        agent_event: None,
        origin: Some(method::ORIGIN_DISCOVERED.to_owned()),
        location_hint: None,
        // Present exactly when the ledger holds a derivation for the Session
        // this runtime was found to be carrying. A runtime nothing has named
        // yet is no Session, and has nothing to be in a state about.
        attention,
    })
    .unwrap_or_else(|_| serde_json::json!({}))
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

/// Serve `integration.status`, `integration.enable`, and
/// `integration.disable`.
///
/// The daemon performs the operation; a client never writes a provider's
/// configuration itself (ADR 0013 D1). The sequence each operation is —
/// intent recorded, then the file brought to it — lives with the one mutator,
/// which is also what keeps two connections' operations from interleaving.
async fn integration_request(request: &Request, state: &Arc<DaemonState>) -> Dispatch {
    let id = request.id;
    let invalid = |detail: String| ProtocolError::new(ErrorCode::InvalidParams, detail);

    let params: IntegrationParams = match request.params.clone() {
        Some(params) => match serde_json::from_value(params) {
            Ok(params) => params,
            Err(source) => {
                return Dispatch::Reply(Frame::error(id, invalid(source.to_string())));
            }
        },
        None => {
            return Dispatch::Reply(Frame::error(
                id,
                invalid("an integration request needs a provider".to_owned()),
            ));
        }
    };
    let Some(provider) = KnownProvider::from_name(&params.provider) else {
        return Dispatch::Reply(Frame::error(
            id,
            ProtocolError::new(
                ErrorCode::UnknownProvider,
                unknown_provider(&params.provider),
            ),
        ));
    };
    // Corral being unable to name itself is answered as a refusal with its
    // cause, not as a protocol error: the client asked a well-formed question,
    // and "Corral wrote nothing, here is why" is the honest answer to it.
    let (target, relay) = match (
        integration::Target::resolve(provider),
        RelayInvocation::compose_global(provider),
    ) {
        (Ok(target), Ok(relay)) => (target, relay),
        (Err(error), _) => {
            return Dispatch::Reply(Frame::result(
                id,
                unresolvable_wire_value(provider, &error.to_string()),
            ));
        }
        (Ok(_), Err(error)) => {
            return Dispatch::Reply(Frame::result(
                id,
                unresolvable_wire_value(provider, &error.to_string()),
            ));
        }
    };

    let now = SystemTime::now();
    // Every arm's work reads and usually writes a file with an `fsync`, and
    // none of that may happen on the reactor: `corrald` runs one runtime
    // thread, and a synchronous write on it stalls every other connection
    // this daemon is serving.
    let state_dir = state.state_dir().to_path_buf();
    let standing = match request.method.as_str() {
        method::INTEGRATION_STATUS => {
            let (target, relay) = (target.clone(), relay.clone());
            integration::off_the_reactor(move || integration::status(&target, &relay)).await
        }
        // Nothing is published on a store failure: the store can no longer
        // vouch for the intent it was asked to record, and the daemon stops
        // serving rather than answering from it (ADR 0002, Q14).
        method::INTEGRATION_ENABLE => {
            match integration::enable(state, target.clone(), relay.clone(), now, state_dir).await {
                Ok(standing) => standing,
                Err(error) => return Dispatch::FailClosed(error),
            }
        }
        method::INTEGRATION_DISABLE => {
            match integration::disable(state, target.clone(), relay.clone(), now, state_dir).await {
                Ok(standing) => standing,
                Err(error) => return Dispatch::FailClosed(error),
            }
        }
        other => {
            // Unreachable: dispatch routes exactly the three methods above.
            // Answered rather than asserted, because a panic here would take
            // the daemon down over a routing mistake.
            return Dispatch::Reply(Frame::error(
                id,
                ProtocolError::new(ErrorCode::MethodNotFound, format!("{other} is not served")),
            ));
        }
    };

    // The operation did not run at all — the blocking pool is gone, or the
    // work panicked. Answering with a standing would be inventing a fact
    // about the user's configuration; the honest reply is that the daemon
    // could not act.
    let Some(standing) = standing else {
        return Dispatch::Reply(Frame::error(
            id,
            ProtocolError::new(
                ErrorCode::Busy,
                "the daemon could not perform the integration operation; try again",
            ),
        ));
    };

    Dispatch::Reply(Frame::result(
        id,
        integration_wire_value(provider, &target, &standing),
    ))
}

/// The answer when Corral cannot name itself or the file it would write.
fn unresolvable_wire_value(provider: KnownProvider, detail: &str) -> serde_json::Value {
    let result = IntegrationResult {
        provider: provider.as_str().to_owned(),
        standing: method::STANDING_REFUSED.to_owned(),
        claims_delivery: false,
        detail: Some(
            integration::Trigger::NotResolvable {
                detail: detail.to_owned(),
            }
            .to_string(),
        ),
        path: None,
    };
    serde_json::to_value(result).unwrap_or_else(|_| serde_json::json!({}))
}

fn integration_wire_value(
    provider: KnownProvider,
    target: &integration::Target,
    standing: &integration::Standing,
) -> serde_json::Value {
    let (name, detail) = match standing {
        integration::Standing::Installed => (method::STANDING_INSTALLED, None),
        integration::Standing::NotInstalled => (method::STANDING_NOT_INSTALLED, None),
        integration::Standing::Drifted(_) => (
            method::STANDING_DRIFTED,
            Some("Corral's entry is not the one this version writes".to_owned()),
        ),
        integration::Standing::Refused(trigger) => {
            (method::STANDING_REFUSED, Some(trigger.to_string()))
        }
        integration::Standing::RepairWithheld { .. } => (
            method::STANDING_REPAIR_WITHHELD,
            Some(
                "something keeps undoing Corral's integration, so Corral stopped repairing it"
                    .to_owned(),
            ),
        ),
    };
    let result = IntegrationResult {
        provider: provider.as_str().to_owned(),
        standing: name.to_owned(),
        claims_delivery: standing.claims_delivery(),
        detail,
        path: Some(target.path().display().to_string()),
    };
    serde_json::to_value(result).unwrap_or_else(|_| serde_json::json!({}))
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
    let known = known.join(", ");
    // The name came off the wire unvalidated and every client renders this
    // sentence as it stands — the TUI into a full-screen frame it does not
    // re-escape. `ProviderId` is the existing owner of what may be stored,
    // logged, or displayed as a provider name: bounded length, and no
    // character that hides or reorders the text around it. A request that
    // fails it is answered without repeating it back.
    match ProviderId::new(name) {
        Ok(named) => format!(
            "Corral does not know how to start {}. It knows: {known}.",
            named.as_str()
        ),
        Err(_) => {
            format!("Corral does not know how to start what this request named. It knows: {known}.")
        }
    }
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
                provider::LaunchIntent::Fresh { args },
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

    let committed = managed_launch::spawn_and_commit(
        state,
        session,
        run,
        launch,
        geometry,
        injected,
        |began, at| {
            let state = Arc::clone(state);
            let command = command.clone();
            async move {
                state
                    .start_managed_session(command, session, run, began, at)
                    .await
            }
        },
    )
    .await;

    concluded(committed, &command)
}

/// What a managed launch became, in the words a client is answered with.
fn concluded(
    committed: Committed,
    command: &Command,
) -> Result<Concluded, corral_state::StateError> {
    Ok(match committed {
        Committed::Started { session, run } | Committed::Replayed { session, run } => {
            Concluded::Accepted { session, run }
        }
        Committed::NotSpawned(message) => Concluded::Refused {
            code: ErrorCode::InvalidParams,
            message,
        },
        Committed::StoreRefused(error) => return refused_by_store(command, error),
    })
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

/// Perform one `session.resume`, as the owner of its command id.
async fn execute_session_resume(
    resume: ResumeSession,
    state: &Arc<DaemonState>,
) -> Result<Concluded, corral_state::StateError> {
    let ResumeSession { command, session } = resume;

    // The receipt first, and before the per-Session claim, because this answer
    // does not depend on the claim: a command that already executed is replayed
    // from its own durable receipt and spawns nothing. Asking for the claim
    // first would let an unrelated continuation of the same Session, running
    // right now, turn an idempotent retry into `Busy` — a client that lost its
    // response would be told to try later and would never reach the Run it
    // already made (ADR 0002 D4).
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

    // Held from here past the commit. Two *different* command ids continuing
    // one Session would otherwise both find nothing running and both spawn,
    // putting two provider processes on one conversation — which the version
    // matrix records a provider will happily allow (grill Q7). Each has its own
    // receipt, so the check above never answers for the other one.
    let Some(_continuing) = state.claim_continuation(session) else {
        return Ok(Concluded::Refused {
            code: ErrorCode::Busy,
            message: "Corral is already continuing this session".to_owned(),
        });
    };

    let plan = match resume_plan(state, session).await {
        Ok(Ok(plan)) => plan,
        Ok(Err(refused)) => {
            return Ok(Concluded::Refused {
                // Three answers, because a client does three different things
                // with them: send it again, ask a different daemon about the
                // agent, or read what the Session's own state says. None of
                // them is `invalid_params` — the parameters were fine, and a
                // client sent looking for a mistake in its request would not
                // find one.
                code: match refused {
                    ResumeRefused::RuntimeUnavailable => ErrorCode::Busy,
                    ResumeRefused::UnknownProvider(_) => ErrorCode::UnknownProvider,
                    ResumeRefused::NotThisDaemon
                    | ResumeRefused::IdentityUnknown
                    | ResumeRefused::Eligibility(_)
                    | ResumeRefused::RunStillLive
                    | ResumeRefused::EndUnverifiable
                    | ResumeRefused::NoPreviousRun
                    | ResumeRefused::EpisodeOrderUnknown => ErrorCode::SessionNotContinuable,
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
        provider::LaunchIntent::Continue {
            external_id: plan.external_id.clone(),
        },
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
    let geometry = PtyGeometry::expect_valid(24, 80);
    let committed = managed_launch::spawn_and_commit(
        state,
        session,
        run,
        launch,
        geometry,
        injected,
        |began, at| {
            let state = Arc::clone(state);
            let command = command.clone();
            async move {
                state
                    .resume_managed_session(command, session, run, began, at)
                    .await
            }
        },
    )
    .await;

    concluded(committed, &command)
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
