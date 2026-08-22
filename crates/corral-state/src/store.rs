use std::path::Path;
use std::time::SystemTime;

use corral_core::{
    Binding, BindingId, BindingKey, BindingKind, Command, CommandOutcome, CommandReceipt,
    CorralSessionId, Evidence, EvidenceSource, NodeId, OccurrenceTime, Provenance, Run, RunEnd,
    RunId, RunOrdinal, Session, SessionLineage,
};
use rusqlite::{Connection, Transaction, TransactionBehavior};

use crate::encoding;
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
///
/// A write decides and records inside one transaction: what it read to reach
/// its decision cannot change under it, so an idempotent path stays idempotent
/// when two writers race.
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

/// How large a command fingerprint may be.
///
/// The canonical form is stored whole, so a conflict can be read rather than
/// guessed at — but a durable row is not a place for unbounded client input. A
/// command whose semantic inputs are genuinely large fingerprints a digest of
/// them instead, which the accepted semantics allow (ADR 0002, Q12).
const FINGERPRINT_LIMIT: usize = 4096;

/// What a write concluded: the answer for the caller, and the facts to append.
struct Written<T> {
    answer: T,
    facts: Vec<SessionEvent>,
}

impl<T> Written<T> {
    /// A write that found the work already done. Nothing is appended, and the
    /// answer is what the store already held.
    fn nothing_to_record(answer: T) -> Self {
        Self {
            answer,
            facts: Vec::new(),
        }
    }

    fn recording(answer: T, facts: Vec<SessionEvent>) -> Self {
        Self { answer, facts }
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

    /// Confirm the store can still vouch for durable truth.
    ///
    /// What every other operation does first, on its own — for a caller whose
    /// answer is a claim about the registry rather than a fact out of it. An
    /// empty list is such a claim.
    pub fn vouch(&mut self) -> Result<(), StateError> {
        self.read(|_| Ok(()))
    }

    pub fn sessions(&mut self) -> Result<Vec<Session>, StateError> {
        self.read(projection::sessions)
    }

    pub fn session(&mut self, id: CorralSessionId) -> Result<Option<Session>, StateError> {
        self.read(|connection| projection::session(connection, id))
    }

    pub fn binding(&mut self, id: BindingId) -> Result<Option<Binding>, StateError> {
        self.read(|connection| projection::binding(connection, id))
    }

    pub fn bindings_of(&mut self, session: CorralSessionId) -> Result<Vec<Binding>, StateError> {
        self.read(|connection| projection::bindings_of(connection, session))
    }

    pub fn runs_of(&mut self, session: CorralSessionId) -> Result<Vec<Run>, StateError> {
        self.read(|connection| projection::runs_of(connection, session))
    }

    pub fn lineage_of(
        &mut self,
        child: CorralSessionId,
    ) -> Result<Option<SessionLineage>, StateError> {
        self.read(|connection| projection::lineage_of(connection, child))
    }

    pub fn receipt(
        &mut self,
        command: &corral_core::CommandId,
    ) -> Result<Option<CommandReceipt>, StateError> {
        self.read(|connection| projection::receipt(connection, command))
    }

    /// One Session's stream, oldest fact first.
    pub fn events_of(
        &mut self,
        session: CorralSessionId,
    ) -> Result<Vec<RecordedEvent>, StateError> {
        self.read(|connection| read_events(connection, Some(session)))
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
        let at = encoding::as_stored(at)?;
        let length = command.fingerprint().as_str().len();
        if length > FINGERPRINT_LIMIT {
            return Err(Refusal::FingerprintTooLarge {
                length,
                limit: FINGERPRINT_LIMIT,
            }
            .into());
        }

        self.write(|transaction| {
            if let Some(receipt) = projection::receipt(transaction, command.id())? {
                if receipt.fingerprint() == command.fingerprint() {
                    return Ok(Written::nothing_to_record(CommandAcceptance::Replayed(
                        receipt,
                    )));
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
            Ok(Written::recording(
                CommandAcceptance::Executed(receipt),
                vec![
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
                ],
            ))
        })
    }

    /// Resolve an external identity to its Session, creating both the Session
    /// and the binding when the identity is new.
    ///
    /// This is what discovery performs, and the lookup and the creation share
    /// one transaction, so a re-scan racing a first scan cannot produce two
    /// Sessions for one external identity.
    pub fn resolve_or_create_session(
        &mut self,
        key: BindingKey,
        provenance: Provenance,
        evidence: Evidence,
        at: SystemTime,
    ) -> Result<SessionResolution, StateError> {
        let at = encoding::as_stored(at)?;
        let evidence = as_stored_evidence(evidence)?;

        self.write(move |transaction| {
            if let Some(binding) = projection::binding_by_key(transaction, &key)? {
                let session =
                    projection::session(transaction, binding.session())?.ok_or_else(|| {
                        FatalState::Unreadable {
                            detail: format!(
                                "binding {} names session {}, which the projections do not hold",
                                binding.id(),
                                binding.session()
                            ),
                        }
                    })?;
                return Ok(Written::nothing_to_record(SessionResolution::Existing {
                    session,
                    binding,
                }));
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
            Ok(Written::recording(
                SessionResolution::Created {
                    session,
                    binding: binding.clone(),
                },
                vec![
                    SessionEvent::SessionCreated {
                        session: session.id(),
                        created_at: at,
                    },
                    SessionEvent::BindingAdded(binding),
                ],
            ))
        })
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
        let at = encoding::as_stored(at)?;
        let evidence = as_stored_evidence(evidence)?;

        self.write(move |transaction| {
            if let Some(binding) = projection::binding_by_key(transaction, &key)? {
                return Ok(Written::nothing_to_record(BindingResolution::Existing(
                    binding,
                )));
            }

            let binding = Binding::new(BindingId::mint(), session, key, provenance, evidence, at);
            refuse_second_control_capable_runtime_binding(transaction, &binding)?;
            Ok(Written::recording(
                BindingResolution::Created(binding.clone()),
                vec![SessionEvent::BindingAdded(binding)],
            ))
        })
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
        let evidence = as_stored_evidence(evidence)?;

        self.write(move |transaction| {
            let confirmed = require_binding(transaction, binding)?.with_evidence(evidence);
            refuse_second_control_capable_runtime_binding(transaction, &confirmed)?;
            Ok(Written::recording(
                confirmed.clone(),
                vec![SessionEvent::BindingConfirmed {
                    session: confirmed.session(),
                    binding,
                    evidence,
                }],
            ))
        })
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
        let started = as_stored_occurrence(started)?;

        self.write(move |transaction| {
            let binding = require_binding(transaction, runtime_binding)?;
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
            // One runtime runs one episode at a time. The log can only speak
            // for the Runs it holds — a Run withheld as heuristic is the
            // caller's live state, and staying out of this question is the
            // same reason it stays out of the log.
            if let Some(live) = projection::live_run_of_binding(transaction, runtime_binding)? {
                return Err(Refusal::RunAlreadyLive {
                    binding: runtime_binding,
                    run: live,
                }
                .into());
            }

            let session = binding.session();
            let run = Run::started(RunId::mint(), session, runtime_binding, started);
            if !binding.assurance().may_assert_durable_fact() {
                // Unnumbered on purpose: the store numbers the Runs it keeps,
                // and a number it has not kept would name a position another
                // Run already occupies.
                return Ok(Written::nothing_to_record(RecordedRun {
                    run,
                    durability: Durability::Withheld,
                }));
            }

            let ordinal = RunOrdinal::from_position(
                projection::run_count(transaction, session)?.saturating_add(1),
            );
            let run = run.with_ordinal(ordinal);
            Ok(Written::recording(
                RecordedRun {
                    run: run.clone(),
                    durability: Durability::Recorded,
                },
                vec![SessionEvent::RunStarted {
                    session,
                    run: run.id(),
                    runtime_binding,
                    ordinal,
                    started_at: started.authoritative(),
                }],
            ))
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
        let at = as_stored_occurrence(at)?;
        let id = run.id();

        self.write(move |transaction| {
            let Some(recorded) = projection::recorded_run(transaction, id)? else {
                return Ok(Written::nothing_to_record(Durability::Withheld));
            };
            // An episode ends once. Recording a second end would overwrite a
            // fact the log already states, and "exited" quietly becoming
            // "unverifiable" is exactly the rewriting the log exists to
            // prevent.
            if !recorded.is_live() {
                return Err(Refusal::RunAlreadyEnded(id).into());
            }
            Ok(Written::recording(
                Durability::Recorded,
                vec![SessionEvent::RunEnded {
                    session: recorded.session(),
                    run: id,
                    end,
                    ended_at: at.authoritative(),
                }],
            ))
        })
    }

    /// A runtime binding became available for this Run.
    pub fn record_run_attached(
        &mut self,
        run: &Run,
        at: SystemTime,
    ) -> Result<Durability, StateError> {
        let at = encoding::as_stored(at)?;
        let id = run.id();
        self.record_run_fact(id, move |session| SessionEvent::RunAttached {
            session,
            run: id,
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
        let at = encoding::as_stored(at)?;
        let id = run.id();
        self.record_run_fact(id, move |session| SessionEvent::RunDetached {
            session,
            run: id,
            at,
        })
    }

    /// Record that one Session continued another.
    ///
    /// The edge's assurance was settled when it was constructed: heuristic
    /// similarity cannot produce a `SessionLineage` at all, so no guessed
    /// edge can reach the log (ADR 0002 D4).
    pub fn record_fork(&mut self, lineage: SessionLineage) -> Result<(), StateError> {
        self.write(move |_| {
            Ok(Written::recording(
                (),
                vec![SessionEvent::SessionForkedFrom(lineage)],
            ))
        })
    }

    /// Rebuild every projection from the log.
    ///
    /// The log owns durable truth; the projections only summarize it. If this
    /// does not reproduce what was there, a projection acquired a fact the log
    /// does not hold, which is an architecture violation rather than a repair
    /// job (ADR 0002 D6).
    pub fn rebuild_projections(&mut self) -> Result<(), StateError> {
        self.write(|transaction| {
            projection::clear(transaction)?;
            replay(transaction)?;
            Ok(Written::nothing_to_record(()))
        })
    }

    /// Record a fact about a Run, if the log knows the Run at all.
    ///
    /// The Session comes from the log's own record of the Run rather than from
    /// the caller's copy, so a fact can never be filed under a Session the
    /// store does not agree the Run belongs to.
    fn record_run_fact(
        &mut self,
        run: RunId,
        event: impl FnOnce(CorralSessionId) -> SessionEvent,
    ) -> Result<Durability, StateError> {
        self.write(move |transaction| {
            let Some(recorded) = projection::recorded_run(transaction, run)? else {
                return Ok(Written::nothing_to_record(Durability::Withheld));
            };
            Ok(Written::recording(
                Durability::Recorded,
                vec![event(recorded.session())],
            ))
        })
    }

    fn read<T>(
        &mut self,
        work: impl FnOnce(&Connection) -> Result<T, StateError>,
    ) -> Result<T, StateError> {
        if let Some(fatal) = &self.fatal {
            return Err(StateError::Fatal(fatal.clone()));
        }
        let outcome =
            schema::vouch(&self.connection, self.node).and_then(|()| work(&self.connection));
        self.guard(outcome)
    }

    fn write<T>(
        &mut self,
        work: impl FnOnce(&Transaction<'_>) -> Result<Written<T>, StateError>,
    ) -> Result<T, StateError> {
        if let Some(fatal) = &self.fatal {
            return Err(StateError::Fatal(fatal.clone()));
        }
        let outcome = transact(&mut self.connection, self.node, work);
        self.guard(outcome)
    }

    /// Remember a fatal conclusion, so nothing after it is answered normally.
    ///
    /// Only a fatal one: a refusal leaves the store exactly as it was, and
    /// latching those would let one rejected write end the daemon.
    fn guard<T>(&mut self, outcome: Result<T, StateError>) -> Result<T, StateError> {
        if let Err(StateError::Fatal(fatal)) = &outcome {
            self.fatal.get_or_insert_with(|| fatal.clone());
        }
        outcome
    }
}

/// Decide and record in one transaction.
///
/// Immediate rather than deferred: a write takes the writer lock before it
/// reads, so it cannot decide on state another writer changes before the
/// commit — and cannot deadlock trying to upgrade a read lock it already
/// holds. Vouching happens inside it too, so the identity a write trusted is
/// the identity it wrote under.
fn transact<T>(
    connection: &mut Connection,
    node: NodeId,
    work: impl FnOnce(&Transaction<'_>) -> Result<Written<T>, StateError>,
) -> Result<T, StateError> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    schema::vouch(&transaction, node)?;
    let Written { answer, facts } = work(&transaction)?;
    append(&transaction, &facts)?;
    transaction.commit()?;
    Ok(answer)
}

/// Append facts and update the projections they justify.
fn append(transaction: &Transaction<'_>, facts: &[SessionEvent]) -> Result<(), StateError> {
    if facts.is_empty() {
        return Ok(());
    }
    let recorded_at = encoding::millis(SystemTime::now())?;
    for event in facts {
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
        projection::apply(transaction, event)?;
    }
    Ok(())
}

fn require_binding(connection: &Connection, id: BindingId) -> Result<Binding, StateError> {
    projection::binding(connection, id)?.ok_or_else(|| Refusal::UnknownBinding(id).into())
}

/// At most one control-capable runtime binding is active per Session.
///
/// Supersession has no producer and no accepted event, so the second
/// acquisition fails closed rather than quietly displacing the first: a
/// projection may not learn a fact the log cannot express (ADR 0002, Q15).
fn refuse_second_control_capable_runtime_binding(
    connection: &Connection,
    candidate: &Binding,
) -> Result<(), StateError> {
    if !candidate.is_control_capable_runtime_binding() {
        return Ok(());
    }
    let existing = projection::bindings_of(connection, candidate.session())?
        .into_iter()
        .find(|binding| {
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

/// An instant as the store will read it back, so a value handed to a caller is
/// the value a later read produces.
fn as_stored_evidence(evidence: Evidence) -> Result<Evidence, StateError> {
    Ok(Evidence::new(
        evidence.source(),
        evidence.assurance(),
        encoding::as_stored(evidence.observed_at())?,
    ))
}

fn as_stored_occurrence(at: OccurrenceTime) -> Result<OccurrenceTime, StateError> {
    Ok(match at {
        OccurrenceTime::Authoritative(instant) => {
            OccurrenceTime::Authoritative(encoding::as_stored(instant)?)
        }
        // Never written, so never rounded: only what the store keeps has to
        // match what it will read back.
        other => other,
    })
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
    let rows = statement.query_map(rusqlite::params_from_iter(parameters), decode_row)?;

    let mut events = Vec::new();
    for row in rows {
        let (session, seq, kind, payload) = row?;
        events.push(recorded_event(session, seq, &kind, &payload)?);
    }
    Ok(events)
}

/// Replay the whole log into the projections, one fact at a time.
///
/// Streamed rather than collected: the log is append-only and never
/// compacted, so materializing it would make the peak cost of a rebuild grow
/// with the account's whole history — on the one operation whose purpose is
/// recovery.
fn replay(transaction: &Transaction<'_>) -> Result<(), StateError> {
    let mut statement = transaction
        .prepare("SELECT session_id, seq, kind, payload FROM session_events ORDER BY global_seq")?;
    let rows = statement.query_map([], decode_row)?;
    for row in rows {
        let (session, seq, kind, payload) = row?;
        let recorded = recorded_event(session, seq, &kind, &payload)?;
        projection::apply(transaction, &recorded.event)?;
    }
    Ok(())
}

type EventRow = (String, i64, String, String);

fn decode_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<EventRow> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
}

fn recorded_event(
    session: String,
    seq: i64,
    kind: &str,
    payload: &str,
) -> Result<RecordedEvent, StateError> {
    let payload: serde_json::Value =
        serde_json::from_str(payload).map_err(|source| FatalState::Unreadable {
            detail: source.to_string(),
        })?;
    Ok(RecordedEvent {
        seq: u64::try_from(seq).map_err(|_| FatalState::Unreadable {
            detail: format!("sequence {seq} is not a position in a stream"),
        })?,
        event: event::decode(session.parse().map_err(FatalState::from)?, kind, &payload)?,
    })
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
