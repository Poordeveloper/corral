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

    /// Whether this store has concluded it can no longer vouch for durable
    /// truth. Once true, never false.
    ///
    /// The conclusion outlives whichever caller reached it: a task cancelled
    /// mid-shutdown cannot take it with it, which is what makes this the thing
    /// an exit status is read from.
    #[must_use]
    pub fn stopped_vouching(&self) -> bool {
        self.fatal.is_some()
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
        self.read(|connection| read_events(connection, session))
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
        // Inside the write, not before it: a store that has already concluded
        // it cannot vouch must answer that, not a refusal a caller would read
        // as "still fine, try again".
        self.write(|transaction| {
            let at = encoding::as_stored(at)?;
            let length = command.fingerprint().as_str().len();
            if length > FINGERPRINT_LIMIT {
                return Err(Refusal::FingerprintTooLarge {
                    length,
                    limit: FINGERPRINT_LIMIT,
                }
                .into());
            }

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
        self.write(move |transaction| {
            let at = encoding::as_stored(at)?;
            let evidence = as_stored_evidence(evidence)?;
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
        let outcome = self.write(move |transaction| {
            let at = encoding::as_stored(at)?;
            let evidence = as_stored_evidence(evidence)?;
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
        });
        self.name_unknown_session(outcome, session)
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
        self.write(move |transaction| {
            let evidence = as_stored_evidence(evidence)?;
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
        self.write(move |transaction| {
            let started = as_stored_occurrence(started)?;
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

            // Saturating rather than wrapping: past four billion Runs the
            // number stops being useful, and a wrapped one would name a Run at
            // the top of the list.
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
        let id = run.id();

        self.write(move |transaction| {
            let at = as_stored_occurrence(at)?;
            let Some(recorded) = live_run_to_record(transaction, id)? else {
                return Ok(Written::nothing_to_record(Durability::Withheld));
            };
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
        let id = run.id();
        self.record_run_fact(id, move |session| {
            Ok(SessionEvent::RunAttached {
                session,
                run: id,
                at: encoding::as_stored(at)?,
            })
        })
    }

    /// A runtime binding stopped being available. Not the end of the Run:
    /// closing a surface never terminates managed work.
    pub fn record_run_detached(
        &mut self,
        run: &Run,
        at: SystemTime,
    ) -> Result<Durability, StateError> {
        let id = run.id();
        self.record_run_fact(id, move |session| {
            Ok(SessionEvent::RunDetached {
                session,
                run: id,
                at: encoding::as_stored(at)?,
            })
        })
    }

    /// Record that one Session continued another.
    ///
    /// The edge's assurance was settled when it was constructed: heuristic
    /// similarity cannot produce a `SessionLineage` at all, so no guessed
    /// edge can reach the log (ADR 0002 D4).
    pub fn record_fork(&mut self, lineage: SessionLineage) -> Result<(), StateError> {
        let outcome = self.write(move |transaction| {
            refuse_lineage_cycle(transaction, lineage)?;
            // A Session's origin is recorded once. Re-recording the same one is
            // a retry and does nothing; a different one is a conflict named as
            // such, rather than a primary-key message a caller has to read a
            // second time to interpret.
            if let Some(recorded) = projection::lineage_of(transaction, lineage.child())? {
                if recorded == lineage {
                    return Ok(Written::nothing_to_record(()));
                }
                return Err(Refusal::LineageAlreadyRecorded {
                    child: recorded.child(),
                    parent: recorded.parent(),
                    assurance: recorded.assurance(),
                }
                .into());
            }
            Ok(Written::recording(
                (),
                vec![SessionEvent::SessionForkedFrom(lineage)],
            ))
        });
        self.name_unknown_session(outcome, lineage.child())
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

    /// Record a fact about a Run, if the log holds a live Run to record it
    /// against.
    ///
    /// The Session comes from the log's own record of the Run rather than from
    /// the caller's copy, so a fact can never be filed under a Session the
    /// store does not agree the Run belongs to.
    fn record_run_fact(
        &mut self,
        run: RunId,
        event: impl FnOnce(CorralSessionId) -> Result<SessionEvent, StateError>,
    ) -> Result<Durability, StateError> {
        self.write(move |transaction| {
            let Some(recorded) = live_run_to_record(transaction, run)? else {
                return Ok(Written::nothing_to_record(Durability::Withheld));
            };
            Ok(Written::recording(
                Durability::Recorded,
                vec![event(recorded.session())?],
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
        let outcome = read_under_one_vouch(&mut self.connection, self.node, work);
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

    /// Give a rolled-back referential-integrity failure the name it deserves.
    ///
    /// The store's own foreign keys are what reject a Session that is not
    /// there. A Rust pre-check would give one invariant two owners, and the
    /// rollback the constraint triggers is what proves a fact and the
    /// projection it justifies share a transaction. Naming it afterwards costs
    /// one lookup, on a path that has already failed.
    fn name_unknown_session<T>(
        &mut self,
        outcome: Result<T, StateError>,
        session: CorralSessionId,
    ) -> Result<T, StateError> {
        if matches!(
            &outcome,
            Err(StateError::Refused(Refusal::Constraint { .. }))
        ) && self.session(session)?.is_none()
        {
            return Err(Refusal::UnknownSession(session).into());
        }
        outcome
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

/// Vouch and read inside one transaction.
///
/// Deferred rather than immediate: a read takes no writer lock. It does take a
/// transaction, so the store cannot be replaced between the vouch and the read
/// it authorizes — which is the only thing vouching is for.
fn read_under_one_vouch<T>(
    connection: &mut Connection,
    node: NodeId,
    work: impl FnOnce(&Connection) -> Result<T, StateError>,
) -> Result<T, StateError> {
    let transaction = connection.transaction()?;
    schema::vouch(&transaction, node)?;
    let answer = work(&transaction)?;
    transaction.commit()?;
    Ok(answer)
}

/// Refuse lineage that would close a loop.
///
/// The log is append-only and PR2 accepts no correction event, so a cycle
/// written once could never be removed, and every consumer walking ancestry
/// would have to invent its own depth cap. `SessionLineage::record` already
/// refuses the one-step case for exactly this reason; only the store can see
/// the longer ones.
fn refuse_lineage_cycle(connection: &Connection, edge: SessionLineage) -> Result<(), StateError> {
    let cycle = || {
        StateError::from(Refusal::LineageWouldCycle {
            child: edge.child(),
            parent: edge.parent(),
        })
    };
    let mut ancestor = edge.parent();
    let mut seen = std::collections::HashSet::new();
    while seen.insert(ancestor) {
        if ancestor == edge.child() {
            return Err(cycle());
        }
        match projection::lineage_of(connection, ancestor)? {
            Some(edge) => ancestor = edge.parent(),
            None => return Ok(()),
        }
    }
    // The stored chain already loops, so nothing may be hung off it.
    Err(cycle())
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

/// The live Run this fact belongs to, or `None` when the log holds no Run to
/// record it against.
///
/// The store keeps no record of a Run it withheld, so it cannot tell one from
/// a Run it was never told about — and must not try. The binding's assurance
/// now says nothing about what it was when that Run started, so consulting it
/// here would refuse the ordinary discover → confirm → exit sequence, where a
/// Run whose start was withheld quite correctly keeps its later facts out of
/// the log too (ADR 0002 D6).
fn live_run_to_record(connection: &Connection, run: RunId) -> Result<Option<Run>, StateError> {
    match projection::recorded_run(connection, run)? {
        Some(recorded) if recorded.is_live() => Ok(Some(recorded)),
        // An episode ends once, and attachment cannot follow it. A second fact
        // here would contradict the outcome the log already states, and the log
        // is never rewritten.
        Some(_) => Err(Refusal::RunAlreadyEnded(run).into()),
        None => Ok(None),
    }
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
    let existing = projection::control_capable_runtime_binding(
        connection,
        candidate.session(),
        candidate.id(),
    )?;
    match existing {
        None => Ok(()),
        Some(existing) => Err(Refusal::ControlCapableRuntimeBindingExists {
            session: candidate.session(),
            existing,
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
    session: CorralSessionId,
) -> Result<Vec<RecordedEvent>, StateError> {
    let mut statement = connection.prepare_cached(&format!(
        "{EVENT_COLUMNS} WHERE session_id = ?1 ORDER BY global_seq"
    ))?;
    let rows = statement.query_map(rusqlite::params![session.to_string()], decode_row)?;

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
    let mut statement = transaction.prepare(&format!("{EVENT_COLUMNS} ORDER BY global_seq"))?;
    let rows = statement.query_map([], decode_row)?;
    for row in rows {
        let (session, seq, kind, payload) = row?;
        let recorded = recorded_event(session, seq, &kind, &payload)?;
        projection::apply(transaction, &recorded.event)?;
    }
    Ok(())
}

/// One shape for the log's rows, so a column added later is added once.
const EVENT_COLUMNS: &str = "SELECT session_id, seq, kind, payload FROM session_events";

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
