//! The durable semantic event log's vocabulary and its encoding.
//!
//! These are the Corral-owned facts the registry store orders, replays, and
//! keeps consistent. The set is founder-accepted and closed: a change the
//! vocabulary cannot express is out of scope until the owning phase extends it
//! (AGENTS.md §Durable state; ADR 0002 D6).
//!
//! Nothing derived and nothing runtime-owned appears here — no PTY bytes, no
//! raw hook events, no provider transcripts, no status.

use std::str::FromStr;
use std::time::SystemTime;

use corral_core::{
    Assurance, Binding, BindingId, BindingKey, CommandFingerprint, CommandId, CommandOutcome,
    CorralSessionId, Evidence, ExternalId, MalformedId, ProviderId, RunEnd, RunId, RunOrdinal,
    SessionLineage,
};
use serde_json::{Value, json};

use crate::encoding::{
    assurance_from_token, assurance_token, binding_kind_from_token, binding_kind_token,
    command_outcome_is, command_outcome_token, evidence_source_from_token, evidence_source_token,
    from_millis, millis, provenance_from_token, provenance_token, run_end_from_token,
    run_end_token, unreadable,
};
use crate::error::FatalState;

/// One accepted fact.
///
/// Occurrence times are plain instants here rather than the domain's
/// `OccurrenceTime`, and that is the point: only an authoritative occurrence
/// time ever reaches this type. A first-observed instant is live metadata and
/// is dropped at the store boundary, so nothing downstream can read one as a
/// start time (ADR 0002 D6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionEvent {
    SessionCreated {
        session: CorralSessionId,
        created_at: SystemTime,
    },
    BindingAdded(Binding),
    BindingConfirmed {
        session: CorralSessionId,
        binding: BindingId,
        evidence: Evidence,
    },
    /// The process episode began.
    RunStarted {
        session: CorralSessionId,
        run: RunId,
        runtime_binding: BindingId,
        ordinal: RunOrdinal,
        /// Absent when the runtime itself cannot say when it began.
        started_at: Option<SystemTime>,
    },
    /// The process episode ended, or could not be established to have ended.
    RunEnded {
        session: CorralSessionId,
        run: RunId,
        end: RunEnd,
        ended_at: Option<SystemTime>,
    },
    /// A runtime binding became available. A different fact from the episode
    /// beginning: `Started, Attached, Detached, Attached, Detached, Ended` is
    /// a legal history, which is what "closing a surface never terminates
    /// managed work" means in the log (ADR 0002 D6).
    RunAttached {
        session: CorralSessionId,
        run: RunId,
        at: SystemTime,
    },
    RunDetached {
        session: CorralSessionId,
        run: RunId,
        at: SystemTime,
    },
    SessionForkedFrom(SessionLineage),
    CommandAccepted {
        command: CommandId,
        fingerprint: CommandFingerprint,
        outcome: CommandOutcome,
        accepted_at: SystemTime,
    },
}

impl SessionEvent {
    /// Which Session's stream this fact belongs to.
    ///
    /// A fork is recorded on the child: the new Session's own stream is what
    /// states where it came from. A command is recorded on the Session it
    /// produced — a future command that mutates no Session needs a
    /// node-scoped stream, which is a durable-schema decision for the phase
    /// that adds one.
    #[must_use]
    pub fn session(&self) -> CorralSessionId {
        match self {
            Self::SessionCreated { session, .. }
            | Self::BindingConfirmed { session, .. }
            | Self::RunStarted { session, .. }
            | Self::RunEnded { session, .. }
            | Self::RunAttached { session, .. }
            | Self::RunDetached { session, .. } => *session,
            Self::BindingAdded(binding) => binding.session(),
            Self::SessionForkedFrom(lineage) => lineage.child(),
            Self::CommandAccepted { outcome, .. } => match outcome {
                CommandOutcome::SessionCreated(session) => *session,
            },
        }
    }

    /// The durable name of this kind of fact. Permanent once written.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::SessionCreated { .. } => "session-created",
            Self::BindingAdded(_) => "binding-added",
            Self::BindingConfirmed { .. } => "binding-confirmed",
            Self::RunStarted { .. } => "run-started",
            Self::RunEnded { .. } => "run-ended",
            Self::RunAttached { .. } => "run-attached",
            Self::RunDetached { .. } => "run-detached",
            Self::SessionForkedFrom(_) => "session-forked-from",
            Self::CommandAccepted { .. } => "command-accepted",
        }
    }
}

/// The stored payload of a fact. The Session id is a column of the log, not a
/// payload field, so it is never written twice and can never disagree.
pub(crate) fn encode(event: &SessionEvent) -> Result<Value, FatalState> {
    let payload = match event {
        SessionEvent::SessionCreated { created_at, .. } => json!({
            "created_at_ms": millis(*created_at)?,
        }),
        SessionEvent::BindingAdded(binding) => json!({
            "binding_id": binding.id().to_string(),
            "node_id": binding.key().node().to_string(),
            "kind": binding_kind_token(binding.kind()),
            "provider": binding.key().provider().as_str(),
            "external_id": binding.key().external_id().as_str(),
            "provenance": provenance_token(binding.provenance()),
            "assurance": assurance_token(binding.assurance()),
            "evidence_source": evidence_source_token(binding.evidence().source()),
            "observed_at_ms": millis(binding.evidence().observed_at())?,
            "created_at_ms": millis(binding.created_at())?,
        }),
        SessionEvent::BindingConfirmed {
            binding, evidence, ..
        } => json!({
            "binding_id": binding.to_string(),
            "assurance": assurance_token(evidence.assurance()),
            "evidence_source": evidence_source_token(evidence.source()),
            "observed_at_ms": millis(evidence.observed_at())?,
        }),
        SessionEvent::RunStarted {
            run,
            runtime_binding,
            ordinal,
            started_at,
            ..
        } => json!({
            "run_id": run.to_string(),
            "runtime_binding_id": runtime_binding.to_string(),
            "ordinal": ordinal.position(),
            "started_at_ms": optional_millis(*started_at)?,
        }),
        SessionEvent::RunEnded {
            run, end, ended_at, ..
        } => json!({
            "run_id": run.to_string(),
            "end_state": run_end_token(*end),
            "ended_at_ms": optional_millis(*ended_at)?,
        }),
        SessionEvent::RunAttached { run, at, .. } | SessionEvent::RunDetached { run, at, .. } => {
            json!({
                "run_id": run.to_string(),
                "at_ms": millis(*at)?,
            })
        }
        SessionEvent::SessionForkedFrom(lineage) => json!({
            "parent_session_id": lineage.parent().to_string(),
            "assurance": assurance_token(lineage.assurance()),
        }),
        SessionEvent::CommandAccepted {
            command,
            fingerprint,
            outcome,
            accepted_at,
        } => {
            let CommandOutcome::SessionCreated(created) = outcome;
            json!({
                "command_id": command.as_str(),
                "fingerprint": fingerprint.as_str(),
                "outcome_kind": command_outcome_token(*outcome),
                "outcome_target": created.to_string(),
                "accepted_at_ms": millis(*accepted_at)?,
            })
        }
    };
    Ok(payload)
}

/// Read a stored fact back.
///
/// Two kinds of future input, two defined answers. An unknown *kind* is
/// unreadable: a fact this build cannot interpret would leave every projection
/// derived from the log silently incomplete, so it fails closed. An unknown
/// *field* inside a kind this build knows is ignored: a payload may gain a
/// field without the fact changing meaning, and refusing one would make a
/// store unreadable by the build that wrote it the moment anything is added.
/// A field whose meaning does change is a new kind, not a new field.
pub(crate) fn decode(
    session: CorralSessionId,
    kind: &str,
    payload: &Value,
) -> Result<SessionEvent, FatalState> {
    let event = match kind {
        "session-created" => SessionEvent::SessionCreated {
            session,
            created_at: from_millis(integer(payload, "created_at_ms")?),
        },
        "binding-added" => SessionEvent::BindingAdded(Binding::new(
            identity(payload, "binding_id")?,
            session,
            BindingKey::new(
                identity(payload, "node_id")?,
                binding_kind_from_token(text(payload, "kind")?)?,
                ProviderId::new(text(payload, "provider")?).map_err(|error| {
                    FatalState::Unreadable {
                        detail: error.to_string(),
                    }
                })?,
                ExternalId::new(text(payload, "external_id")?).map_err(|error| {
                    FatalState::Unreadable {
                        detail: error.to_string(),
                    }
                })?,
            ),
            provenance_from_token(text(payload, "provenance")?)?,
            evidence(payload)?,
            from_millis(integer(payload, "created_at_ms")?),
        )),
        "binding-confirmed" => SessionEvent::BindingConfirmed {
            session,
            binding: identity(payload, "binding_id")?,
            evidence: evidence(payload)?,
        },
        "run-started" => SessionEvent::RunStarted {
            session,
            run: identity(payload, "run_id")?,
            runtime_binding: identity(payload, "runtime_binding_id")?,
            ordinal: RunOrdinal::from_position(
                u32::try_from(integer(payload, "ordinal")?)
                    .map_err(|_| unreadable("a run ordinal", "an out-of-range integer"))?,
            ),
            started_at: optional_instant(payload, "started_at_ms")?,
        },
        "run-ended" => SessionEvent::RunEnded {
            session,
            run: identity(payload, "run_id")?,
            end: run_end_from_token(text(payload, "end_state")?)?,
            ended_at: optional_instant(payload, "ended_at_ms")?,
        },
        "run-attached" => SessionEvent::RunAttached {
            session,
            run: identity(payload, "run_id")?,
            at: from_millis(integer(payload, "at_ms")?),
        },
        "run-detached" => SessionEvent::RunDetached {
            session,
            run: identity(payload, "run_id")?,
            at: from_millis(integer(payload, "at_ms")?),
        },
        "session-forked-from" => SessionEvent::SessionForkedFrom(
            SessionLineage::record(
                session,
                identity(payload, "parent_session_id")?,
                assurance(payload)?,
            )
            .map_err(|refusal| FatalState::Unreadable {
                detail: refusal.to_string(),
            })?,
        ),
        "command-accepted" => {
            command_outcome_is(text(payload, "outcome_kind")?)?;
            SessionEvent::CommandAccepted {
                command: CommandId::new(text(payload, "command_id")?).map_err(|error| {
                    FatalState::Unreadable {
                        detail: error.to_string(),
                    }
                })?,
                fingerprint: CommandFingerprint::from_canonical(text(payload, "fingerprint")?),
                outcome: CommandOutcome::SessionCreated(identity(payload, "outcome_target")?),
                accepted_at: from_millis(integer(payload, "accepted_at_ms")?),
            }
        }
        other => return Err(unreadable("a recorded fact this build knows", other)),
    };
    Ok(event)
}

fn evidence(payload: &Value) -> Result<Evidence, FatalState> {
    Ok(Evidence::new(
        evidence_source_from_token(text(payload, "evidence_source")?)?,
        assurance(payload)?,
        from_millis(integer(payload, "observed_at_ms")?),
    ))
}

fn assurance(payload: &Value) -> Result<Assurance, FatalState> {
    assurance_from_token(text(payload, "assurance")?)
}

fn text<'a>(payload: &'a Value, field: &str) -> Result<&'a str, FatalState> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| unreadable(&format!("a payload with a text {field}"), "something else"))
}

fn integer(payload: &Value, field: &str) -> Result<i64, FatalState> {
    payload.get(field).and_then(Value::as_i64).ok_or_else(|| {
        unreadable(
            &format!("a payload with a numeric {field}"),
            "something else",
        )
    })
}

/// A null field is a recorded absence — "this happened, and the runtime could
/// not say when" — and never a zero instant.
fn optional_instant(payload: &Value, field: &str) -> Result<Option<SystemTime>, FatalState> {
    match payload.get(field) {
        Some(Value::Null) | None => Ok(None),
        Some(_) => Ok(Some(from_millis(integer(payload, field)?))),
    }
}

fn optional_millis(at: Option<SystemTime>) -> Result<Option<i64>, FatalState> {
    at.map(millis).transpose()
}

fn identity<T: FromStr<Err = MalformedId>>(payload: &Value, field: &str) -> Result<T, FatalState> {
    text(payload, field)?.parse().map_err(FatalState::from)
}

#[cfg(test)]
#[path = "event_tests.rs"]
mod tests;
