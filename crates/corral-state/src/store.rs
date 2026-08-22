use std::path::Path;
use std::time::SystemTime;

use corral_core::{
    Binding, BindingId, BindingKey, BindingKind, Command, CommandOutcome, CommandReceipt,
    CorralSessionId, Evidence, EvidenceSource, NodeId, OccurrenceTime, Provenance, Run, RunEnd,
    RunId, RunOrdinal, Session, SessionLineage,
};
use rusqlite::Connection;

use crate::error::{FatalState, Refusal, StateError};
use crate::event::{self, SessionEvent};
use crate::projection;
use crate::schema;

/// The registry store: Corral-owned facts, and the projections summarizing
/// them.
///
/// Every operation first confirms the store is still the schema and the store
/// this process validated at startup. Once it concludes it cannot vouch for
/// durable truth, it never answers normally again — a normal-looking
/// projection from an untrusted store is worse than no answer (ADR 0002, Q14).
pub struct Store {
    connection: Connection,
    node: NodeId,
    fatal: Option<FatalState>,
}

/// Whether a lifecycle fact entered the durable log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Durability {
    /// Appended.
    Recorded,
    /// Deliberately not written, and the Run is unaffected by that: durability
    /// follows fact assurance, not object existence. Writing `RunStarted` into
    /// a Session's stream asserts the Run belongs to it, and under a Heuristic
    /// runtime binding that assertion is a guess. A Run whose start was
    /// withheld keeps its later facts out of the log too, because confirming
    /// an association later never promotes earlier heuristic runtime metadata
    /// into durable truth (ADR 0002 D6).
    Withheld,
}

/// A Run, and whether the log took its start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedRun {
    run: Run,
    durability: Durability,
}

impl RecordedRun {
    /// The Run. It exists whether or not the log took it.
    #[must_use]
    pub fn run(&self) -> &Run {
        &self.run
    }

    #[must_use]
    pub fn durability(&self) -> Durability {
        self.durability
    }
}

/// What resolving an external identity found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionResolution {
    /// The identity was already known, and this is the Session it names.
    Existing {
        session: Session,
        binding: Binding,
    },
    Created {
        session: Session,
        binding: Binding,
    },
}

/// What attaching an external identity to a known Session found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BindingResolution {
    Created(Binding),
    /// The identity was already bound. The existing binding is authoritative,
    /// including when it names a Session the caller did not expect: binding
    /// uniqueness is what stops one external identity resolving to two
    /// Sessions (`ARCHITECTURE.md` §1).
    Existing(Binding),
}

/// What a mutating command did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandAcceptance {
    Executed(CommandReceipt),
    /// The same semantic command already ran under this id. Its original
    /// receipt is returned and nothing was executed a second time.
    Replayed(CommandReceipt),
}

impl CommandAcceptance {
    #[must_use]
    pub fn receipt(&self) -> &CommandReceipt {
        match self {
            Self::Executed(receipt) | Self::Replayed(receipt) => receipt,
        }
    }
}

impl Store {
    /// Open the registry store, or conclude it cannot be used.
    pub fn open(path: &Path) -> Result<Self, StateError> {
        let (connection, node) = schema::open(path)?;
        Ok(Self {
            connection,
            node,
            fatal: None,
        })
    }

    /// The node this store belongs to. Minted on first open and never
    /// re-derived: it scopes every external binding, so a node that forgot its
    /// own identity would rediscover its own sessions as somebody else's.
    #[must_use]
    pub fn node(&self) -> NodeId {
        self.node
    }

    pub fn sessions(&mut self) -> Result<Vec<Session>, StateError> {
        self.vouch()?;
        let outcome = projection::sessions(&self.connection);
        self.guard(outcome)
    }

    pub fn binding(&mut self, id: BindingId) -> Result<Option<Binding>, StateError> {
        self.vouch()?;
        let outcome = projection::binding(&self.connection, id);
        self.guard(outcome)
    }

    pub fn bindings_of(&mut self, session: CorralSessionId) -> Result<Vec<Binding>, StateError> {
        self.vouch()?;
        let outcome = projection::bindings_of(&self.connection, session);
        self.guard(outcome)
    }

    pub fn runs_of(&mut self, session: CorralSessionId) -> Result<Vec<Run>, StateError> {
        self.vouch()?;
        let outcome = projection::runs_of(&self.connection, session);
        self.guard(outcome)
    }

    pub fn lineage_of(
        &mut self,
        child: CorralSessionId,
    ) -> Result<Option<SessionLineage>, StateError> {
        self.vouch()?;
        let outcome = projection::lineage_of(&self.connection, child);
        self.guard(outcome)
    }

    pub fn receipt(
        &mut self,
        command: &corral_core::CommandId,
    ) -> Result<Option<CommandReceipt>, StateError> {
        self.vouch()?;
        let outcome = projection::receipt(&self.connection, command);
        self.guard(outcome)
    }

    /// One Session's stream, oldest fact first.
    pub fn events_of(
        &mut self,
        session: CorralSessionId,
    ) -> Result<Vec<RecordedEvent>, StateError> {
        self.vouch()?;
        let outcome = read_events(&self.connection, Some(session));
        self.guard(outcome)
    }

    /// Create a Session under a client-supplied command id.
    ///
    /// Reuse of the id with the same semantic command returns the original
    /// receipt without creating a second Session; reuse with a different one
    /// is a conflict that executes nothing and leaves the receipt untouched
    /// (ADR 0002, Q12).
    pub fn create_session(
        &mut self,
        command: &Command,
        at: SystemTime,
    ) -> Result<CommandAcceptance, StateError> {
        self.vouch()?;
        let existing = {
            let outcome = projection::receipt(&self.connection, command.id());
            self.guard(outcome)?
        };
        if let Some(receipt) = existing {
            if receipt.fingerprint() == command.fingerprint() {
                return Ok(CommandAcceptance::Replayed(receipt));
            }
            return Err(Refusal::CommandIdConflict {
                command: command.id().clone(),
            }
            .into());
        }

        let session = CorralSessionId::mint();
        let receipt = CommandReceipt::new(
            command.id().clone(),
            command.fingerprint().clone(),
            CommandOutcome::SessionCreated(session),
            at,
        );
        self.commit(&[
            SessionEvent::SessionCreated {
                session,
                created_at: at,
            },
            SessionEvent::CommandAccepted {
                command: command.id().clone(),
                fingerprint: command.fingerprint().clone(),
                outcome: CommandOutcome::SessionCreated(session),
                accepted_at: at,
            },
        ])?;
        Ok(CommandAcceptance::Executed(receipt))
    }

    /// Resolve an external identity to its Session, creating both the Session
    /// and the binding when the identity is new.
    ///
    /// This is what discovery performs, and it is one transaction so that a
    /// re-scan racing a first scan cannot produce two Sessions for one
    /// external identity.
    pub fn resolve_or_create_session(
        &mut self,
        key: BindingKey,
        provenance: Provenance,
        evidence: Evidence,
        at: SystemTime,
    ) -> Result<SessionResolution, StateError> {
        self.vouch()?;
        let existing = {
            let outcome = projection::binding_by_key(&self.connection, &key);
            self.guard(outcome)?
        };
        if let Some(binding) = existing {
            let session = self.session_of(binding.session())?;
            return Ok(SessionResolution::Existing { session, binding });
        }

        let session = Session::new(CorralSessionId::mint(), at);
        let binding = Binding::new(
            BindingId::mint(),
            session.id(),
            key,
            provenance,
            evidence,
            at,
        );
        self.commit(&[
            SessionEvent::SessionCreated {
                session: session.id(),
                created_at: at,
            },
            SessionEvent::BindingAdded(binding.clone()),
        ])?;
        Ok(SessionResolution::Created { session, binding })
    }

    /// Attach an external identity to a Session Corral already has.
    pub fn bind(
        &mut self,
        session: CorralSessionId,
        key: BindingKey,
        provenance: Provenance,
        evidence: Evidence,
        at: SystemTime,
    ) -> Result<BindingResolution, StateError> {
        self.vouch()?;
        let existing = {
            let outcome = projection::binding_by_key(&self.connection, &key);
            self.guard(outcome)?
        };
        if let Some(binding) = existing {
            return Ok(BindingResolution::Existing(binding));
        }

        let binding = Binding::new(BindingId::mint(), session, key, provenance, evidence, at);
        self.refuse_second_control_capable_runtime_binding(&binding)?;
        self.commit(&[SessionEvent::BindingAdded(binding.clone())])?;
        Ok(BindingResolution::Created(binding))
    }

    /// Replace the evidence supporting a binding.
    ///
    /// Assurance is re-evaluated when evidence changes, so this is where a
    /// heuristic association becomes one Corral may act on — and where the
    /// facts it was withholding become writable.
    pub fn confirm_binding(
        &mut self,
        binding: BindingId,
        evidence: Evidence,
    ) -> Result<Binding, StateError> {
        self.vouch()?;
        let confirmed = self.require_binding(binding)?.with_evidence(evidence);
        self.refuse_second_control_capable_runtime_binding(&confirmed)?;
        self.commit(&[SessionEvent::BindingConfirmed {
            session: confirmed.session(),
            binding,
            evidence,
        }])?;
        Ok(confirmed)
    }

    /// Open a Run under the Session its runtime binding names.
    ///
    /// The Session is taken from the binding rather than from the caller: a
    /// Run's association *is* its runtime binding, and a Run that could name a
    /// Session its binding does not would be a second, weaker association with
    /// no assurance behind it (ADR 0002, Q8).
    ///
    /// `occurrence` is why the caller believes a concrete runtime occurrence
    /// exists, which is a different question from how sure it is of the
    /// association — an Attested binding rests on a hook naming the provider
    /// session, and a hook having fired is not a runtime being alive. Only
    /// construction or node-local runtime observation mints a Run (D2).
    pub fn record_run_started(
        &mut self,
        runtime_binding: BindingId,
        occurrence: EvidenceSource,
        started: OccurrenceTime,
    ) -> Result<RecordedRun, StateError> {
        self.vouch()?;
        let binding = self.require_binding(runtime_binding)?;
        if binding.kind() != BindingKind::Runtime {
            return Err(Refusal::NotARuntimeBinding(runtime_binding).into());
        }
        if !occurrence.establishes_runtime_occurrence() {
            return Err(Refusal::EvidenceCannotMintARun {
                binding: runtime_binding,
                source: occurrence,
            }
            .into());
        }

        let session = binding.session();
        let ordinal = {
            let outcome = projection::run_count(&self.connection, session);
            RunOrdinal::from_position(self.guard(outcome)?.saturating_add(1))
        };
        let run = Run::started(RunId::mint(), session, runtime_binding, ordinal, started);

        if !binding.assurance().may_assert_durable_fact() {
            return Ok(RecordedRun {
                run,
                durability: Durability::Withheld,
            });
        }
        self.commit(&[SessionEvent::RunStarted {
            session,
            run: run.id(),
            runtime_binding,
            ordinal,
            started_at: started.authoritative(),
        }])?;
        Ok(RecordedRun {
            run,
            durability: Durability::Recorded,
        })
    }

    /// Close a Run.
    ///
    /// The Run is the caller's live state; the store decides only whether the
    /// fact may be written. An end that cannot be established is recorded as
    /// unverifiable, never as an exit.
    pub fn record_run_ended(
        &mut self,
        run: &Run,
        end: RunEnd,
        at: OccurrenceTime,
    ) -> Result<Durability, StateError> {
        self.vouch()?;
        let Some(recorded) = self.recorded_run(run.id())? else {
            return Ok(Durability::Withheld);
        };
        // An episode ends once. Recording a second end would overwrite a fact
        // the log already states, and "exited" quietly becoming
        // "unverifiable" is exactly the rewriting the log exists to prevent.
        if !recorded.is_live() {
            return Err(Refusal::RunAlreadyEnded(run.id()).into());
        }
        self.commit(&[SessionEvent::RunEnded {
            session: recorded.session(),
            run: run.id(),
            end,
            ended_at: at.authoritative(),
        }])?;
        Ok(Durability::Recorded)
    }

    /// A runtime binding became available for this Run.
    pub fn record_run_attached(
        &mut self,
        run: &Run,
        at: SystemTime,
    ) -> Result<Durability, StateError> {
        self.record_run_fact(run, |session| SessionEvent::RunAttached {
            session,
            run: run.id(),
            at,
        })
    }

    /// A runtime binding stopped being available. Not the end of the Run:
    /// closing a surface never terminates managed work.
    pub fn record_run_detached(
        &mut self,
        run: &Run,
        at: SystemTime,
    ) -> Result<Durability, StateError> {
        self.record_run_fact(run, |session| SessionEvent::RunDetached {
            session,
            run: run.id(),
            at,
        })
    }

    /// Record that one Session continued another.
    ///
    /// The edge's assurance was settled when it was constructed: heuristic
    /// similarity cannot produce a `SessionLineage` at all, so no guessed
    /// edge can reach the log (ADR 0002 D4).
    pub fn record_fork(&mut self, lineage: SessionLineage) -> Result<(), StateError> {
        self.vouch()?;
        self.commit(&[SessionEvent::SessionForkedFrom(lineage)])
    }

    /// Rebuild every projection from the log.
    ///
    /// The log owns durable truth; the projections only summarize it. If this
    /// does not reproduce what was there, a projection acquired a fact the log
    /// does not hold, which is an architecture violation rather than a repair
    /// job (ADR 0002 D6).
    pub fn rebuild_projections(&mut self) -> Result<(), StateError> {
        self.vouch()?;
        let recorded = {
            let outcome = read_events(&self.connection, None);
            self.guard(outcome)?
        };
        let outcome = replay(&mut self.connection, &recorded);
        self.guard(outcome)
    }

    /// Record a fact about a Run, if the log knows the Run at all.
    ///
    /// The Session comes from the log's own record of the Run rather than from
    /// the caller's copy, so a fact can never be filed under a Session the
    /// store does not agree the Run belongs to.
    fn record_run_fact(
        &mut self,
        run: &Run,
        event: impl FnOnce(CorralSessionId) -> SessionEvent,
    ) -> Result<Durability, StateError> {
        self.vouch()?;
        let Some(recorded) = self.recorded_run(run.id())? else {
            return Ok(Durability::Withheld);
        };
        self.commit(&[event(recorded.session())])?;
        Ok(Durability::Recorded)
    }

    fn recorded_run(&mut self, run: RunId) -> Result<Option<Run>, StateError> {
        let outcome = projection::recorded_run(&self.connection, run);
        self.guard(outcome)
    }

    fn require_binding(&mut self, id: BindingId) -> Result<Binding, StateError> {
        let outcome = projection::binding(&self.connection, id);
        self.guard(outcome)?
            .ok_or_else(|| Refusal::UnknownBinding(id).into())
    }

    fn session_of(&mut self, id: CorralSessionId) -> Result<Session, StateError> {
        let outcome = projection::sessions(&self.connection);
        self.guard(outcome)?
            .into_iter()
            .find(|session| session.id() == id)
            .ok_or_else(|| {
                FatalState::Unreadable {
                    detail: format!(
                        "binding names session {id}, which the projections do not hold"
                    ),
                }
                .into()
            })
    }

    /// At most one control-capable runtime binding is active per Session.
    ///
    /// Supersession has no producer and no accepted event, so the second
    /// acquisition fails closed rather than quietly displacing the first: a
    /// projection may not learn a fact the log cannot express (ADR 0002, Q15).
    fn refuse_second_control_capable_runtime_binding(
        &mut self,
        candidate: &Binding,
    ) -> Result<(), StateError> {
        if !candidate.is_control_capable_runtime_binding() {
            return Ok(());
        }
        let outcome = projection::bindings_of(&self.connection, candidate.session());
        let existing = self.guard(outcome)?.into_iter().find(|binding| {
            binding.id() != candidate.id() && binding.is_control_capable_runtime_binding()
        });
        match existing {
            None => Ok(()),
            Some(existing) => Err(Refusal::ControlCapableRuntimeBindingExists {
                session: candidate.session(),
                existing: existing.id(),
            }
            .into()),
        }
    }

    fn commit(&mut self, events: &[SessionEvent]) -> Result<(), StateError> {
        let outcome = commit(&mut self.connection, events);
        self.guard(outcome)
    }

    fn vouch(&mut self) -> Result<(), StateError> {
        if let Some(fatal) = &self.fatal {
            return Err(StateError::Fatal(fatal.clone()));
        }
        let outcome = schema::vouch(&self.connection, self.node);
        self.guard(outcome)
    }

    /// Remember a fatal conclusion, so nothing after it is answered normally.
    fn guard<T>(&mut self, outcome: Result<T, StateError>) -> Result<T, StateError> {
        if let Err(StateError::Fatal(fatal)) = &outcome {
            self.fatal.get_or_insert_with(|| fatal.clone());
        }
        outcome
    }
}

/// Append facts and update the projections they justify — one transaction, so
/// a failure anywhere leaves neither.
fn commit(connection: &mut Connection, events: &[SessionEvent]) -> Result<(), StateError> {
    let recorded_at = crate::encoding::millis(SystemTime::now())?;
    let transaction = connection.transaction()?;
    for event in events {
        let session = event.session().to_string();
        // The per-Session sequence is the order Corral accepted facts about
        // that Session. It only ever grows: a fact learned late is appended
        // now, never inserted into an earlier position (ADR 0002 D6).
        let seq: i64 = transaction.query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM session_events WHERE session_id = ?1",
            [&session],
            |row| row.get(0),
        )?;
        transaction.execute(
            "INSERT INTO session_events (session_id, seq, kind, payload, recorded_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                session,
                seq,
                event.kind(),
                event::encode(event)?.to_string(),
                recorded_at,
            ],
        )?;
        projection::apply(&transaction, event)?;
    }
    transaction.commit()?;
    Ok(())
}

/// One fact as the log holds it: what happened, and where it sits in its
/// Session's acceptance order.
///
/// The sequence is the order Corral *accepted* the fact, which is not when the
/// fact happened. A `RunStarted` accepted now may carry an occurrence twenty
/// minutes old, and reading the two as one number is the confusion ADR 0002 D6
/// forbids.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedEvent {
    seq: u64,
    event: SessionEvent,
}

impl RecordedEvent {
    #[must_use]
    pub fn seq(&self) -> u64 {
        self.seq
    }

    #[must_use]
    pub fn event(&self) -> &SessionEvent {
        &self.event
    }
}

/// Facts in the order Corral accepted them.
///
/// `global_seq` orders across Sessions, which a replay must follow: a lineage
/// edge names a Session that was created in another stream, and per-Session
/// order alone would not place the two.
fn read_events(
    connection: &Connection,
    session: Option<CorralSessionId>,
) -> Result<Vec<RecordedEvent>, StateError> {
    let (filter, parameters): (&str, Vec<String>) = match session {
        Some(session) => ("WHERE session_id = ?1", vec![session.to_string()]),
        None => ("", Vec::new()),
    };
    let mut statement = connection.prepare(&format!(
        "SELECT session_id, seq, kind, payload FROM session_events {filter} ORDER BY global_seq"
    ))?;
    let rows = statement.query_map(rusqlite::params_from_iter(parameters), |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;

    let mut events = Vec::new();
    for row in rows {
        let (session, seq, kind, payload) = row?;
        let payload: serde_json::Value =
            serde_json::from_str(&payload).map_err(|source| FatalState::Unreadable {
                detail: source.to_string(),
            })?;
        events.push(RecordedEvent {
            seq: u64::try_from(seq).map_err(|_| FatalState::Unreadable {
                detail: format!("sequence {seq} is not a position in a stream"),
            })?,
            event: event::decode(session.parse().map_err(FatalState::from)?, &kind, &payload)?,
        });
    }
    Ok(events)
}

fn replay(connection: &mut Connection, events: &[RecordedEvent]) -> Result<(), StateError> {
    let transaction = connection.transaction()?;
    projection::clear(&transaction)?;
    for recorded in events {
        projection::apply(&transaction, &recorded.event)?;
    }
    transaction.commit()?;
    Ok(())
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
