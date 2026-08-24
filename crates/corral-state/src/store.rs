use std::path::Path;
use std::time::SystemTime;

use corral_core::{
    Assurance, Binding, BindingId, BindingKey, BindingKind, Command, CommandOutcome,
    CommandReceipt, CorralSessionId, Evidence, EvidenceSource, NodeId, OccurrenceTime, Provenance,
    ReservedNamespace, Run, RunEnd, RunId, Session, SessionLineage,
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
    /// runtime binding that assertion is a guess.
    ///
    /// A Run whose start was withheld keeps its later facts out of the log
    /// too: confirming an association never *automatically* promotes earlier
    /// heuristic runtime metadata into durable truth. What it does is make the
    /// promotion possible — the runtime owner brings the Run back through
    /// `record_withheld_run_started`, asserting that authoritative evidence
    /// still supports it (ADR 0002 D6).
    Withheld,
}

/// What the caller knows about a Run the store withheld.
///
/// Not the `Run` itself: the closure that records it outlives the borrow, and
/// these are the only parts of one the store may act on.
struct WithheldRun {
    session: CorralSessionId,
    end: Option<(RunEnd, OccurrenceTime)>,
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
    ///
    /// The evidence the call carried is not written over the binding's. Every
    /// durable projection change needs an accepted event behind it (ADR 0002
    /// D6), and a rescan is not one — re-evaluating evidence in live state is
    /// unrestricted, and strengthening the recorded binding is
    /// `confirm_binding`, said out loud.
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
    /// The identity was already bound to the Session the caller named, so the
    /// link it asked for is the link that exists. An identity another Session
    /// holds is refused instead — binding uniqueness is what stops one
    /// external identity resolving to two Sessions (`ARCHITECTURE.md` §1).
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

/// What opening a managed session under a command produced.
///
/// The acceptance carries the receipt and says whether this call executed or
/// replayed; the Session and Run are what the caller answers with, and are the
/// same on both paths — a replay names what the first execution made rather
/// than a second Session nobody asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartedManagedSession {
    acceptance: CommandAcceptance,
    session: CorralSessionId,
    run: RunId,
}

impl StartedManagedSession {
    #[must_use]
    pub fn acceptance(&self) -> &CommandAcceptance {
        &self.acceptance
    }

    #[must_use]
    pub fn session(&self) -> CorralSessionId {
        self.session
    }

    #[must_use]
    pub fn run(&self) -> RunId {
        self.run
    }

    /// Whether this call performed the command rather than answering from a
    /// receipt.
    ///
    /// The caller's runtime is only real on the executed path: a replay must
    /// never leave a second process running under one command id.
    #[must_use]
    pub fn executed(&self) -> bool {
        matches!(self.acceptance, CommandAcceptance::Executed(_))
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

    /// Open a managed session under a client-supplied command id: the Session,
    /// the runtime binding Corral owns for it, and its first Run — one
    /// accepted command, one transaction.
    ///
    /// The four facts are inseparable. A receipt written without its Run would
    /// name a Session whose episode nothing can describe, and a retry after a
    /// crash in that window could only answer with an outcome the accepted
    /// vocabulary has no variant for. So either the whole command happened or
    /// none of it did, and a retry that finds no receipt is a legitimate retry
    /// (grill Q8, Q9).
    ///
    /// Reuse of the id with the same semantic command replays what it made,
    /// executing nothing; reuse with a different one is a conflict that leaves
    /// the receipt untouched (ADR 0002, Q12).
    ///
    /// `run` is minted by the caller, before the runtime it names exists.
    /// Minting an id is not asserting that a runtime exists — the caller
    /// reaches here only once its `spawn` has confirmed one, and a spawn that
    /// failed simply leaves the id unused (grill Q3).
    pub fn start_managed_session(
        &mut self,
        command: &Command,
        run: RunId,
        started: OccurrenceTime,
        at: SystemTime,
    ) -> Result<StartedManagedSession, StateError> {
        let node = self.node;
        // Inside the write, not before it: a store that has already concluded
        // it cannot vouch must answer that, not a refusal a caller would read
        // as "still fine, try again".
        self.write(move |transaction| {
            let at = encoding::as_stored(at)?;
            let started = as_stored_occurrence(started)?;
            let length = command.fingerprint().as_str().len();
            if length > FINGERPRINT_LIMIT {
                return Err(Refusal::FingerprintTooLarge {
                    length,
                    limit: FINGERPRINT_LIMIT,
                }
                .into());
            }

            if let Some(replayed) = already_started(transaction, command)? {
                return Ok(Written::nothing_to_record(replayed));
            }

            let session = CorralSessionId::mint();
            // The shape is the store's, not the caller's: ADR 0008 D1 fixes
            // what a managed runtime binding is, and a caller that could
            // supply its own provenance or assurance could create a managed
            // runtime the durable rules do not describe.
            let binding = Binding::new(
                BindingId::mint(),
                session,
                BindingKey::mint_managed_runtime(node),
                Provenance::CorralCreated,
                as_stored_evidence(Evidence::new(
                    EvidenceSource::CorralConstructed,
                    Assurance::Deterministic,
                    at,
                ))?,
                at,
            );
            refuse_reserved_namespace(&binding)?;
            let receipt = CommandReceipt::new(
                command.id().clone(),
                command.fingerprint().clone(),
                CommandOutcome::SessionCreated(session),
                at,
            );
            // The two rules `start_run` enforces hold here by construction:
            // Corral built this runtime, so the occurrence is
            // `CorralConstructed` and the association is `Deterministic`.
            // Nothing a caller passes can weaken either.
            Ok(Written::recording(
                StartedManagedSession {
                    acceptance: CommandAcceptance::Executed(receipt),
                    session,
                    run,
                },
                vec![
                    SessionEvent::SessionCreated {
                        session,
                        created_at: at,
                    },
                    SessionEvent::BindingAdded(binding.clone()),
                    SessionEvent::RunStarted {
                        session,
                        run,
                        runtime_binding: binding.id(),
                        started_at: started.authoritative(),
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

    /// What this command already did, if it has run before.
    ///
    /// The consult that has to happen *before* a second runtime side effect is
    /// allowed. Spawning first and discovering afterwards that this command id
    /// was already completed leaves two agents running, the second one nobody
    /// asked for and nobody knows about (grill Q2).
    ///
    /// A read, so it can be made cheaply on every attempt. It is not the whole
    /// answer on its own: two retries arriving at one live daemon can both see
    /// nothing here, which is what the daemon's in-flight table closes.
    pub fn completed_managed_session(
        &mut self,
        command: &Command,
    ) -> Result<Option<StartedManagedSession>, StateError> {
        self.read(|connection| already_started(connection, command))
    }

    /// Close every managed-runtime episode no daemon owns any more.
    ///
    /// Called once at startup, by the daemon that just won the singleton claim
    /// (ADR 0001 D2). At that moment every open managed episode belongs to a
    /// daemon that is gone, and a managed runtime does not survive its owning
    /// daemon (ADR 0007 L6) — which is what makes closing them correct rather
    /// than a guess about processes.
    ///
    /// `Unverifiable`, never an exit: Corral did not watch these end and may
    /// not say that it did. The occurrence time stays unknown for the same
    /// reason — a daemon's startup timestamp is not when a process stopped,
    /// and the event sequence already records when Corral accepted the ending
    /// (grill Q5).
    ///
    /// Scoped to what Corral owns as a managed episode, never to every
    /// unfinished Run on the node: a discovered or provider-owned Run may
    /// legitimately outlive a `corrald` restart.
    ///
    /// No `RunDetached` is fabricated for a viewer that never detached. A
    /// projection reads attachments as inactive after `RunEnded`; it does not
    /// need invented facts to get there (grill Q11).
    pub fn end_unowned_managed_runs(&mut self) -> Result<Vec<RunId>, StateError> {
        let node = self.node;
        self.write(move |transaction| {
            let open = projection::open_managed_runs(transaction, node)?;
            let facts = open
                .iter()
                .map(|(session, run)| SessionEvent::RunEnded {
                    session: *session,
                    run: *run,
                    end: RunEnd::Unverifiable,
                    ended_at: None,
                })
                .collect();
            let closed = open.iter().map(|(_, run)| *run).collect();
            Ok(Written::recording(closed, facts))
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
            refuse_reserved_namespace(&binding)?;
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
                if binding.session() != session {
                    return Err(Refusal::BindingClaimedByAnotherSession {
                        binding: binding.id(),
                        session: binding.session(),
                    }
                    .into());
                }
                return Ok(Written::nothing_to_record(BindingResolution::Existing(
                    binding,
                )));
            }

            let binding = Binding::new(BindingId::mint(), session, key, provenance, evidence, at);
            refuse_reserved_namespace(&binding)?;
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
            let existing = require_binding(transaction, binding)?;
            // A confirmation records evidence strong enough to assert a durable
            // fact. Heuristic evidence is not a confirmation, and writing it
            // would be the assurance-change persistence Q15 deferred — which an
            // append-only log with no correction event could never undo. Not a
            // claim that one level sits below another: Corral does not order
            // assurance at all.
            if !evidence.assurance().may_assert_durable_fact() {
                return Err(Refusal::UnsupportedConfirmation {
                    binding,
                    assurance: evidence.assurance(),
                }
                .into());
            }
            let confirmed = existing.with_evidence(evidence);
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
    ///
    /// The `RunId` is the caller's, minted before the runtime it names
    /// existed. That separation is what lets a process that exits instantly be
    /// recorded correctly: the id is available before the occurrence, and the
    /// durable fact is written only once the occurrence is real (grill Q3).
    pub fn record_run_started(
        &mut self,
        run: RunId,
        runtime_binding: BindingId,
        occurrence: EvidenceSource,
        started: OccurrenceTime,
    ) -> Result<RecordedRun, StateError> {
        self.start_run(run, runtime_binding, None, occurrence, started)
    }

    /// Append the facts of a Run the store withheld while its association was
    /// only heuristic.
    ///
    /// This is D6's other half: they become durable when the association does,
    /// appended then and never inserted into an earlier seq, carrying the
    /// occurrences the runtime still supports. A Run that already ended is
    /// appended whole — start and end in one transaction — because Q10 speaks
    /// of both, and a start committed without its end would leave the log
    /// holding an episode that never finishes.
    ///
    /// The caller brings the Run back rather than a bare id, because the store
    /// kept no record of it and has nothing else to check it against. What it
    /// can check, it does: the Run names the binding the facts are filed
    /// under, and the Session that binding names, so a Run cannot be attached
    /// to an association it does not itself claim. Its own occurrence times
    /// are used, so there is no second source to disagree with it.
    pub fn record_withheld_run_started(
        &mut self,
        run: &Run,
        occurrence: EvidenceSource,
    ) -> Result<RecordedRun, StateError> {
        let withheld = WithheldRun {
            session: run.session(),
            end: run
                .end()
                .map(|end| (end, run.ended_at().unwrap_or(OccurrenceTime::Unknown))),
        };
        self.start_run(
            run.id(),
            run.runtime_binding(),
            Some(withheld),
            occurrence,
            run.started_at(),
        )
    }

    fn start_run(
        &mut self,
        id: RunId,
        runtime_binding: BindingId,
        withheld: Option<WithheldRun>,
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

            let session = binding.session();
            // Asked of every Run, not only the ones brought back: the id is
            // the caller's now, so a repeat is possible on both paths and
            // would otherwise append a second start for one episode.
            if projection::recorded_run(transaction, id)?.is_some() {
                return Err(Refusal::RunAlreadyRecorded(id).into());
            }
            let ending = match withheld {
                None => None,
                Some(withheld) => {
                    if withheld.session != session {
                        return Err(Refusal::RunClaimsAnotherSession {
                            run: id,
                            claimed: withheld.session,
                            binds: session,
                        }
                        .into());
                    }
                    withheld.end
                }
            };

            // One runtime runs one episode at a time — a rule about episodes
            // that overlap. An episode that already ended overlaps nothing, so
            // appending a past one is not blocked by the present one; that is
            // the ordinary shape after a heuristic discovery is confirmed.
            if ending.is_none()
                && let Some(live) = projection::live_run_of_binding(transaction, runtime_binding)?
            {
                return Err(Refusal::RunAlreadyLive {
                    binding: runtime_binding,
                    run: live,
                }
                .into());
            }

            let mut run = Run::started(id, session, runtime_binding, started);
            let mut facts = vec![SessionEvent::RunStarted {
                session,
                run: id,
                runtime_binding,
                started_at: started.authoritative(),
            }];
            if let Some((end, at)) = ending {
                let at = as_stored_occurrence(at)?;
                run = run.ended(end, at);
                facts.push(SessionEvent::RunEnded {
                    session,
                    run: id,
                    end,
                    ended_at: at.authoritative(),
                });
            }

            if !binding.assurance().may_assert_durable_fact() {
                // Unnumbered on purpose: a Run's position is read off the Runs
                // its Session holds, and the store holds none for this one.
                return Ok(Written::nothing_to_record(RecordedRun {
                    run,
                    durability: Durability::Withheld,
                }));
            }
            Ok(Written::recording(
                RecordedRun {
                    run,
                    durability: Durability::Recorded,
                },
                facts,
            ))
        })
    }

    /// Close a Run.
    ///
    /// Named by id rather than by a `Run` value: the store reads the Run back
    /// from its own log anyway, and the runtime owner that establishes an end
    /// holds an id and a fact, not a domain object it would have to assemble
    /// to be allowed to speak.
    ///
    /// An end that cannot be established is recorded as unverifiable, never as
    /// an exit.
    pub fn record_run_ended(
        &mut self,
        id: RunId,
        end: RunEnd,
        at: OccurrenceTime,
    ) -> Result<Durability, StateError> {
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
        id: RunId,
        at: SystemTime,
    ) -> Result<Durability, StateError> {
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
        id: RunId,
        at: SystemTime,
    ) -> Result<Durability, StateError> {
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
            // After the retry branch, never before it: D8 keeps an edge whose
            // parent is deleted later, and a re-record of an edge the store
            // already holds must stay a no-op once delete exists. An edge that
            // never had a parent is a producer bug, and the log cannot take it
            // back.
            if projection::session(transaction, lineage.parent())?.is_none() {
                return Err(Refusal::UnknownSession(lineage.parent()).into());
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
        // The log position this fact just took. The projection orders by it,
        // so what a live write produces and what a replay produces are the
        // same number rather than two guesses at the same idea.
        projection::apply(transaction, event, transaction.last_insert_rowid())?;
    }
    Ok(())
}

/// The live Run this fact belongs to, or `None` when the log holds no Run to
/// record it against.
///
/// The store keeps no record of a Run it withheld, so it cannot tell one from
/// a Run it was never told about — and must not try. The binding's assurance
/// now says nothing about what it was when that Run started, so consulting it
/// here would refuse the ordinary discover → confirm → exit sequence.
///
/// A withheld Run stays out of the log until the runtime owner brings it back
/// through `record_withheld_run_started`; until then, facts about it are
/// withheld too.
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

/// What a command already produced, read from its receipt.
///
/// One implementation for the pre-spawn consult and for the transaction that
/// commits: a replay decided by two different pieces of code is a replay that
/// can disagree with itself.
fn already_started(
    connection: &Connection,
    command: &Command,
) -> Result<Option<StartedManagedSession>, StateError> {
    let Some(receipt) = projection::receipt(connection, command.id())? else {
        return Ok(None);
    };
    if receipt.fingerprint() != command.fingerprint() {
        return Err(Refusal::CommandIdConflict {
            command: command.id().clone(),
        }
        .into());
    }
    let CommandOutcome::SessionCreated(session) = receipt.outcome();
    // A receipt and its Run land in one transaction, so a receipt this command
    // wrote and a Session with no Run cannot both be true. If they are, the log
    // and the projections disagree, and minting a fresh Run to cover it would
    // answer a question about a past that is not there.
    let run =
        projection::first_run_of(connection, session)?.ok_or_else(|| FatalState::Unreadable {
            detail: format!(
                "the receipt for command {} names session {session}, which holds no Run",
                command.id().as_str()
            ),
        })?;
    Ok(Some(StartedManagedSession {
        acceptance: CommandAcceptance::Replayed(receipt),
        session,
        run,
    }))
}

/// Keep the reserved `corral` provider namespace directional (ADR 0008 D3).
///
/// Checked where a binding is created and nowhere else: the namespace records
/// who minted an identity, which is settled at creation. A read path that
/// re-checked it would refuse to load a store rather than refuse to write one,
/// and `STORAGE_EPOCH` is `dev` — a development database that predates this
/// rule is reset, not reinterpreted.
fn refuse_reserved_namespace(binding: &Binding) -> Result<(), StateError> {
    match binding.reserved_namespace() {
        ReservedNamespace::Respected => Ok(()),
        misuse => Err(Refusal::ReservedProviderNamespace {
            binding: binding.id(),
            misuse,
        }
        .into()),
    }
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
        let (_, session, seq, kind, payload) = row?;
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
        let (accepted_seq, session, seq, kind, payload) = row?;
        let recorded = recorded_event(session, seq, &kind, &payload)?;
        projection::apply(transaction, &recorded.event, accepted_seq)?;
    }
    Ok(())
}

/// One shape for the log's rows, so a column added later is added once.
const EVENT_COLUMNS: &str = "SELECT global_seq, session_id, seq, kind, payload FROM session_events";

type EventRow = (i64, String, i64, String, String);

fn decode_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<EventRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
    ))
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
