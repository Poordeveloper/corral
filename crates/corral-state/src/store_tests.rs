use std::fmt::Write as _;
use std::ops::{Deref, DerefMut};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use corral_core::{
    Assurance, BindingKind, CommandFingerprint, CommandId, CommandKind, ControlEligibility,
    EvidenceSource, ExitCause, ExternalId, ProviderId, RunOrdinal,
};

use super::*;

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A registry store on a real file, in a directory of its own.
///
/// Real files rather than in-memory databases: opening, migrating from empty,
/// and reopening after the process that wrote it is gone are the behaviours
/// under test, and an in-memory store has none of them.
struct TestStore {
    store: Store,
    directory: PathBuf,
}

impl TestStore {
    fn new(name: &str) -> Self {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "corral-state-{}-{unique}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("create the scratch directory");
        let store = Store::open(&directory.join(FILE)).expect("open a fresh store");
        Self { store, directory }
    }

    fn path(&self) -> PathBuf {
        self.directory.join(FILE)
    }

    /// Close this store and open the same file again, as a later process
    /// would.
    fn reopen(&mut self) {
        let path = self.path();
        self.store = Store::open(&path).expect("reopen");
    }

    /// A second connection to the same file, standing in for anything that
    /// touches the store behind the daemon's back.
    fn behind_the_daemons_back(&self, statement: &str) {
        // Being the second opener is the whole point here: this stands in for
        // whatever touches the store while the daemon holds it.
        #[allow(clippy::disallowed_methods)]
        let connection = rusqlite::Connection::open(self.path()).expect("open");
        connection.execute(statement, []).expect("execute");
    }
}

const FILE: &str = "registry.sqlite3";

impl Deref for TestStore {
    type Target = Store;

    fn deref(&self) -> &Store {
        &self.store
    }
}

impl DerefMut for TestStore {
    fn deref_mut(&mut self) -> &mut Store {
        &mut self.store
    }
}

impl Drop for TestStore {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn instant(seconds: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
}

fn key(node: NodeId, kind: BindingKind, external: &str) -> BindingKey {
    BindingKey::new(
        node,
        kind,
        ProviderId::new("claude-code").expect("usable"),
        ExternalId::new(external).expect("usable"),
    )
}

fn evidence(source: EvidenceSource, assurance: Assurance) -> Evidence {
    Evidence::new(source, assurance, instant(100))
}

fn owned_runtime() -> Evidence {
    evidence(EvidenceSource::CorralConstructed, Assurance::Deterministic)
}

fn suspected_runtime() -> Evidence {
    evidence(EvidenceSource::NodeRuntimeObservation, Assurance::Heuristic)
}

fn command(id: &str, cwd: &str) -> Command {
    Command::new(
        CommandId::new(id).expect("usable"),
        CommandFingerprint::builder(CommandKind::new("session.create").expect("usable"))
            .input("cwd", cwd)
            .build(),
    )
}

fn kinds(events: &[RecordedEvent]) -> Vec<&'static str> {
    events
        .iter()
        .map(|recorded| recorded.event().kind())
        .collect()
}

/// A Session created under a runtime binding, with the Run the runtime
/// binding names.
fn managed_session(store: &mut Store, external: &str) -> (CorralSessionId, BindingId) {
    let node = store.node();
    let accepted = store
        .create_session(&command(&format!("cmd-{external}"), "/work"), instant(10))
        .expect("created");
    let CommandOutcome::SessionCreated(session) = accepted.receipt().outcome();
    let binding = match store
        .bind(
            session,
            key(node, BindingKind::Runtime, external),
            Provenance::CorralCreated,
            owned_runtime(),
            instant(11),
        )
        .expect("bound")
    {
        BindingResolution::Created(binding) => binding,
        BindingResolution::Existing(binding) => binding,
    };
    (session, binding.id())
}

#[test]
fn a_fresh_store_migrates_from_empty_and_keeps_its_node_identity() {
    let mut store = TestStore::new("fresh");
    let node = store.node();

    assert!(store.path().exists());
    assert_eq!(store.sessions().expect("readable"), Vec::new());

    store.reopen();
    assert_eq!(store.node(), node, "the node identity is minted once");
}

/// A Session's identity never depends on the process that ran it.
#[test]
fn identity_survives_the_process_that_created_it() {
    let mut store = TestStore::new("survives");
    let (session, _) = managed_session(&mut store, "run-a");

    store.reopen();

    let sessions = store.sessions().expect("readable");
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id(), session);
    assert_eq!(store.bindings_of(session).expect("readable").len(), 1);
}

/// Re-scanning resolves a previously seen external identity to its existing
/// Session through binding uniqueness — never to a second Session.
#[test]
fn re_discovery_never_duplicates_a_session() {
    let mut store = TestStore::new("rediscovery");
    let node = store.node();
    let identity = key(node, BindingKind::ProviderSession, "sess-1");

    let first = store
        .resolve_or_create_session(
            identity.clone(),
            Provenance::Discovered,
            evidence(EvidenceSource::ProviderHook, Assurance::Attested),
            instant(10),
        )
        .expect("resolved");
    let second = store
        .resolve_or_create_session(
            identity,
            Provenance::Discovered,
            evidence(EvidenceSource::ProviderHook, Assurance::Attested),
            instant(20),
        )
        .expect("resolved");

    let (SessionResolution::Created { session: first, .. }
    | SessionResolution::Existing { session: first, .. }) = first;
    assert!(
        matches!(second, SessionResolution::Existing { session, .. } if session.id() == first.id())
    );
    assert_eq!(store.sessions().expect("readable").len(), 1);
}

/// A Session may hold one control-capable runtime binding at a time.
///
/// Supersession has no producer and no accepted event (Q15), so the second
/// acquisition fails closed rather than quietly displacing the first — and
/// nothing in PR2 can end the first either. D5's "the previous binding ends or
/// is explicitly superseded before the new one is acquired" therefore has no
/// implementation here: a resumed process cannot yet acquire its own runtime
/// binding, and the phase that owns runtimes brings the event that lets it.
#[test]
fn a_session_holds_at_most_one_control_capable_runtime_binding() {
    let mut store = TestStore::new("one-runtime");
    let node = store.node();
    let (session, first) = managed_session(&mut store, "run-a");

    let refusal = store
        .bind(
            session,
            key(node, BindingKind::Runtime, "run-b"),
            Provenance::CorralCreated,
            owned_runtime(),
            instant(12),
        )
        .expect_err("refused");

    assert!(matches!(
        refusal,
        StateError::Refused(Refusal::ControlCapableRuntimeBindingExists { existing, .. })
            if existing == first
    ));
    assert_eq!(store.bindings_of(session).expect("readable").len(), 1);
}

/// Confirming a second runtime binding acquires control just as adding one
/// does, so it meets the same rule.
#[test]
fn confirming_a_second_runtime_binding_is_refused() {
    let mut store = TestStore::new("confirm-second");
    let node = store.node();
    let (session, _) = managed_session(&mut store, "run-a");
    let BindingResolution::Created(weak) = store
        .bind(
            session,
            key(node, BindingKind::Runtime, "run-b"),
            Provenance::Discovered,
            suspected_runtime(),
            instant(12),
        )
        .expect("bound")
    else {
        panic!("a new external identity is a new binding");
    };

    let refusal = store
        .confirm_binding(
            weak.id(),
            evidence(EvidenceSource::ProviderHook, Assurance::Attested),
        )
        .expect_err("refused");

    assert!(matches!(
        refusal,
        StateError::Refused(Refusal::ControlCapableRuntimeBindingExists { .. })
    ));
    assert_eq!(
        store
            .binding(weak.id())
            .expect("readable")
            .expect("still bound")
            .assurance(),
        Assurance::Heuristic,
        "a refused confirmation changes nothing"
    );
}

/// A runtime observed but only heuristically associated is a Run that exists,
/// under a binding that cannot control. Weak identity never erases the fact
/// that the runtime exists, and never grants control either.
#[test]
fn a_run_exists_under_a_heuristic_binding_and_grants_no_control() {
    let mut store = TestStore::new("heuristic-run");
    let node = store.node();
    let SessionResolution::Created { session, binding } = store
        .resolve_or_create_session(
            key(node, BindingKind::Runtime, "pid-77"),
            Provenance::Discovered,
            suspected_runtime(),
            instant(10),
        )
        .expect("resolved")
    else {
        panic!("a new external identity is a new Session");
    };

    let recorded = store
        .record_run_started(
            binding.id(),
            EvidenceSource::NodeRuntimeObservation,
            OccurrenceTime::Unknown,
        )
        .expect("a Run exists");

    assert_eq!(recorded.run().session(), session.id());
    assert_eq!(
        binding.control_eligibility(),
        ControlEligibility::AssuranceTooWeak,
        "the Run exists, and control still resolves through the binding"
    );
}

/// Writing `RunStarted` into a Session's stream durably asserts the Run
/// belongs to it. Under a Heuristic binding that assertion is a guess, so the
/// fact stays out of the log while the Run itself carries on existing.
#[test]
fn a_heuristically_bound_run_writes_no_durable_lifecycle_fact() {
    let mut store = TestStore::new("withheld");
    let node = store.node();
    let SessionResolution::Created { session, binding } = store
        .resolve_or_create_session(
            key(node, BindingKind::Runtime, "pid-77"),
            Provenance::Discovered,
            suspected_runtime(),
            instant(10),
        )
        .expect("resolved")
    else {
        panic!("a new external identity is a new Session");
    };

    let recorded = store
        .record_run_started(
            binding.id(),
            EvidenceSource::NodeRuntimeObservation,
            OccurrenceTime::Unknown,
        )
        .expect("a Run exists");
    let ended = store
        .record_run_ended(
            recorded.run(),
            RunEnd::Unverifiable,
            OccurrenceTime::Unknown,
        )
        .expect("recorded");

    assert_eq!(recorded.durability(), Durability::Withheld);
    assert_eq!(
        recorded.run().ordinal(),
        None,
        "the store numbers the Runs it keeps"
    );
    assert_eq!(ended, Durability::Withheld);
    assert_eq!(store.runs_of(session.id()).expect("readable"), Vec::new());
    assert_eq!(
        kinds(&store.events_of(session.id()).expect("readable")),
        ["session-created", "binding-added"]
    );
}

/// Facts become durable once the association does — appended then, never
/// inserted into an earlier position, and never by promoting the runtime
/// metadata of a Run whose start was already withheld.
#[test]
fn confirmation_makes_later_facts_durable_without_rewriting_earlier_ones() {
    let mut store = TestStore::new("confirmed");
    let node = store.node();
    let SessionResolution::Created { session, binding } = store
        .resolve_or_create_session(
            key(node, BindingKind::Runtime, "pid-77"),
            Provenance::Discovered,
            suspected_runtime(),
            instant(10),
        )
        .expect("resolved")
    else {
        panic!("a new external identity is a new Session");
    };
    let withheld = store
        .record_run_started(
            binding.id(),
            EvidenceSource::NodeRuntimeObservation,
            OccurrenceTime::Unknown,
        )
        .expect("a Run exists");

    store
        .confirm_binding(
            binding.id(),
            evidence(EvidenceSource::ProviderHook, Assurance::Attested),
        )
        .expect("confirmed");
    let observed_start = instant(400);
    let recorded = store
        .record_run_started(
            binding.id(),
            EvidenceSource::NodeRuntimeObservation,
            OccurrenceTime::Authoritative(observed_start),
        )
        .expect("recorded");

    assert_eq!(recorded.durability(), Durability::Recorded);
    let events = store.events_of(session.id()).expect("readable");
    assert_eq!(
        kinds(&events),
        [
            "session-created",
            "binding-added",
            "binding-confirmed",
            "run-started"
        ],
        "the confirmation is accepted before the fact it unblocks"
    );
    assert!(
        events.windows(2).all(|pair| pair[0].seq() < pair[1].seq()),
        "the sequence only grows"
    );
    let runs = store.runs_of(session.id()).expect("readable");
    assert_eq!(runs.len(), 1, "the withheld Run is not backfilled");
    assert_eq!(runs[0].id(), recorded.run().id());
    assert_ne!(runs[0].id(), withheld.run().id());
}

/// The event sequence is the order Corral accepted a fact; occurrence time is
/// when the fact happened. A fact accepted now may name a much older instant,
/// and that is legal.
#[test]
fn a_late_fact_may_carry_an_earlier_occurrence_time() {
    let mut store = TestStore::new("occurrence");
    let (session, binding) = managed_session(&mut store, "run-a");
    let long_ago = instant(1);

    store
        .record_run_started(
            binding,
            EvidenceSource::CorralConstructed,
            OccurrenceTime::Authoritative(long_ago),
        )
        .expect("recorded");

    let runs = store.runs_of(session).expect("readable");
    assert_eq!(
        runs[0].started_at(),
        OccurrenceTime::Authoritative(long_ago)
    );
    let events = store.events_of(session).expect("readable");
    assert!(
        events.last().expect("a stream").seq() > 1,
        "accepted last, and it happened first"
    );
}

/// If the runtime cannot say when a Run began, nothing is recorded for its
/// start. A first-observed instant is never written as a start time.
#[test]
fn a_first_observed_time_is_never_stored_as_a_start_time() {
    let mut store = TestStore::new("first-observed");
    let (session, binding) = managed_session(&mut store, "run-a");

    store
        .record_run_started(
            binding,
            EvidenceSource::CorralConstructed,
            OccurrenceTime::FirstObserved(instant(500)),
        )
        .expect("recorded");

    let runs = store.runs_of(session).expect("readable");
    assert_eq!(runs[0].started_at(), OccurrenceTime::Unknown);
}

/// A hook event says a provider session exists; it does not say a runtime is
/// alive. Minting a Run from it would turn semantic evidence into runtime
/// truth.
#[test]
fn semantic_evidence_cannot_mint_a_run() {
    let mut store = TestStore::new("semantic");
    let node = store.node();
    let SessionResolution::Created { binding, .. } = store
        .resolve_or_create_session(
            key(node, BindingKind::Runtime, "claimed"),
            Provenance::Discovered,
            evidence(EvidenceSource::ProviderHook, Assurance::Attested),
            instant(10),
        )
        .expect("resolved")
    else {
        panic!("a new external identity is a new Session");
    };

    let refusal = store
        .record_run_started(
            binding.id(),
            EvidenceSource::ProviderHook,
            OccurrenceTime::Unknown,
        )
        .expect_err("refused");

    assert!(matches!(
        refusal,
        StateError::Refused(Refusal::EvidenceCannotMintARun { .. })
    ));
}

#[test]
fn only_a_runtime_binding_can_carry_a_run() {
    let mut store = TestStore::new("not-runtime");
    let node = store.node();
    let SessionResolution::Created { binding, .. } = store
        .resolve_or_create_session(
            key(node, BindingKind::History, "transcript-3"),
            Provenance::Discovered,
            evidence(EvidenceSource::CorralConstructed, Assurance::Deterministic),
            instant(10),
        )
        .expect("resolved")
    else {
        panic!("a new external identity is a new Session");
    };

    let refusal = store
        .record_run_started(
            binding.id(),
            EvidenceSource::NodeRuntimeObservation,
            OccurrenceTime::Unknown,
        )
        .expect_err("refused");

    assert!(matches!(
        refusal,
        StateError::Refused(Refusal::NotARuntimeBinding(_))
    ));
}

/// Resuming a provider session is the same Session with a new Run.
///
/// The identity half of D3/D5, which is the half PR2 owns. The binding half —
/// a new process means a new runtime binding, so the previous one ends or is
/// explicitly superseded — has no producer here and no accepted event; see
/// `a_session_holds_at_most_one_control_capable_runtime_binding`.
#[test]
fn native_resume_opens_a_new_run_under_the_same_session() {
    let mut store = TestStore::new("resume");
    let (session, binding) = managed_session(&mut store, "run-a");
    let first = store
        .record_run_started(
            binding,
            EvidenceSource::CorralConstructed,
            OccurrenceTime::Authoritative(instant(20)),
        )
        .expect("recorded");
    store
        .record_run_ended(
            first.run(),
            RunEnd::Exited(ExitCause::Completed),
            OccurrenceTime::Authoritative(instant(30)),
        )
        .expect("recorded");

    let second = store
        .record_run_started(
            binding,
            EvidenceSource::CorralConstructed,
            OccurrenceTime::Authoritative(instant(40)),
        )
        .expect("recorded");

    assert_eq!(second.run().session(), session);
    assert_eq!(store.sessions().expect("readable").len(), 1);
    let runs = store.runs_of(session).expect("readable");
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].ordinal(), Some(RunOrdinal::FIRST));
    assert_eq!(runs[1].ordinal(), Some(RunOrdinal::from_position(2)));
    assert_eq!(runs[0].end(), Some(RunEnd::Exited(ExitCause::Completed)));
    assert!(runs[1].is_live());
}

/// A process episode ends once. Recording a second end would overwrite the
/// outcome the log already states.
#[test]
fn a_run_ends_once() {
    let mut store = TestStore::new("ends-once");
    let (session, binding) = managed_session(&mut store, "run-a");
    let run = store
        .record_run_started(
            binding,
            EvidenceSource::CorralConstructed,
            OccurrenceTime::Authoritative(instant(20)),
        )
        .expect("recorded");
    store
        .record_run_ended(
            run.run(),
            RunEnd::Exited(ExitCause::Completed),
            OccurrenceTime::Authoritative(instant(30)),
        )
        .expect("recorded");

    let refusal = store
        .record_run_ended(run.run(), RunEnd::Unverifiable, OccurrenceTime::Unknown)
        .expect_err("refused");

    assert!(matches!(
        refusal,
        StateError::Refused(Refusal::RunAlreadyEnded(_))
    ));
    assert_eq!(
        store.runs_of(session).expect("readable")[0].end(),
        Some(RunEnd::Exited(ExitCause::Completed))
    );
}

/// Attachment is a different fact from the episode. Detaching does not end the
/// Run, and neither fact leaves a mark on the projection.
#[test]
fn attachment_is_recorded_without_touching_the_projection() {
    let mut store = TestStore::new("attachment");
    let (session, binding) = managed_session(&mut store, "run-a");
    let run = store
        .record_run_started(
            binding,
            EvidenceSource::CorralConstructed,
            OccurrenceTime::Authoritative(instant(20)),
        )
        .expect("recorded");

    store
        .record_run_attached(run.run(), instant(21))
        .expect("recorded");
    store
        .record_run_detached(run.run(), instant(22))
        .expect("recorded");

    let runs = store.runs_of(session).expect("readable");
    assert_eq!(runs.len(), 1);
    assert!(runs[0].is_live(), "detaching is not an end");
    assert_eq!(
        kinds(&store.events_of(session).expect("readable")),
        [
            "session-created",
            "command-accepted",
            "binding-added",
            "run-started",
            "run-attached",
            "run-detached"
        ]
    );
}

/// Handing context into a fresh provider session produces a new Session with
/// a durable edge — both can be live and independently actionable.
#[test]
fn a_context_handoff_records_a_new_session_with_an_edge() {
    let mut store = TestStore::new("handoff");
    let (parent, _) = managed_session(&mut store, "run-a");
    let (child, _) = managed_session(&mut store, "run-b");

    store
        .record_fork(
            SessionLineage::record(child, parent, Assurance::Deterministic).expect("recordable"),
        )
        .expect("recorded");

    let edge = store.lineage_of(child).expect("readable").expect("an edge");
    assert_eq!(edge.parent(), parent);
    assert_eq!(store.lineage_of(parent).expect("readable"), None);
    assert!(kinds(&store.events_of(child).expect("readable")).contains(&"session-forked-from"));
}

/// An externally observed fork names no parent, so nothing may record one. The
/// edge cannot even be constructed from heuristic similarity, which is what
/// keeps a guess out of the store.
#[test]
fn an_observed_fork_records_no_edge() {
    let mut store = TestStore::new("observed-fork");
    let (parent, _) = managed_session(&mut store, "run-a");
    let (child, _) = managed_session(&mut store, "run-b");

    assert!(SessionLineage::record(child, parent, Assurance::Heuristic).is_err());
    assert_eq!(store.lineage_of(child).expect("readable"), None);
}

/// A binding names a Session the store does not hold, so the projection write
/// fails on the store's own referential integrity — after its event was
/// already written into the transaction. The rollback is what proves the fact
/// and the projection share one.
#[test]
fn a_fact_and_the_projection_it_justifies_commit_together() {
    let mut store = TestStore::new("atomic");
    let node = store.node();
    let ghost = CorralSessionId::mint();
    let identity = key(node, BindingKind::ProviderSession, "sess-1");

    let refusal = store
        .bind(
            ghost,
            identity.clone(),
            Provenance::Discovered,
            evidence(EvidenceSource::ProviderHook, Assurance::Attested),
            instant(10),
        )
        .expect_err("refused");

    assert!(matches!(
        refusal,
        StateError::Refused(Refusal::UnknownSession(_))
    ));
    assert_eq!(
        store.events_of(ghost).expect("readable"),
        Vec::new(),
        "the rolled-back fact left no trace in the log"
    );
    assert!(
        matches!(
            store
                .resolve_or_create_session(
                    identity,
                    Provenance::Discovered,
                    evidence(EvidenceSource::ProviderHook, Assurance::Attested),
                    instant(20),
                )
                .expect("resolved"),
            SessionResolution::Created { .. }
        ),
        "the external identity was never bound"
    );
}

/// Re-recording the same origin is a retry, not a conflict — every other write
/// in the store resolves its own repeat rather than leaving a caller to read a
/// constraint message.
#[test]
fn recording_the_same_origin_twice_records_it_once() {
    let mut store = TestStore::new("fork-retry");
    let (parent, _) = managed_session(&mut store, "run-a");
    let (child, _) = managed_session(&mut store, "run-b");
    let edge = SessionLineage::record(child, parent, Assurance::Deterministic).expect("recordable");

    store.record_fork(edge).expect("recorded");
    store.record_fork(edge).expect("a retry is not a conflict");

    let forks = kinds(&store.events_of(child).expect("readable"))
        .into_iter()
        .filter(|kind| *kind == "session-forked-from")
        .count();
    assert_eq!(forks, 1);
}

/// A Session's origin is recorded once. A different parent would replace a
/// fact rather than add one, and the refusal says so by name.
#[test]
fn a_second_origin_is_refused_by_name() {
    let mut store = TestStore::new("fork-conflict");
    let (first_parent, _) = managed_session(&mut store, "run-a");
    let (other_parent, _) = managed_session(&mut store, "run-b");
    let (child, _) = managed_session(&mut store, "run-c");
    store
        .record_fork(
            SessionLineage::record(child, first_parent, Assurance::Deterministic)
                .expect("recordable"),
        )
        .expect("recorded");

    let refusal = store
        .record_fork(
            SessionLineage::record(child, other_parent, Assurance::Deterministic)
                .expect("recordable"),
        )
        .expect_err("refused");

    assert!(matches!(
        refusal,
        StateError::Refused(Refusal::LineageAlreadyRecorded { parent, .. })
            if parent == first_parent
    ));
    assert_eq!(
        store
            .lineage_of(child)
            .expect("readable")
            .expect("an edge")
            .parent(),
        first_parent
    );
}

/// Attachment is a fact about a live episode. Appending one after the end
/// would record a runtime that exited and then became available — and the log
/// is never rewritten to take it back.
#[test]
fn attachment_cannot_follow_an_end() {
    let mut store = TestStore::new("attach-after-end");
    let (session, binding) = managed_session(&mut store, "run-a");
    let run = store
        .record_run_started(
            binding,
            EvidenceSource::CorralConstructed,
            OccurrenceTime::Authoritative(instant(20)),
        )
        .expect("recorded");
    store
        .record_run_ended(
            run.run(),
            RunEnd::Exited(ExitCause::Completed),
            OccurrenceTime::Authoritative(instant(30)),
        )
        .expect("recorded");

    let refusal = store
        .record_run_attached(run.run(), instant(31))
        .expect_err("refused");

    assert!(matches!(
        refusal,
        StateError::Refused(Refusal::RunAlreadyEnded(_))
    ));
    assert!(
        !kinds(&store.events_of(session).expect("readable")).contains(&"run-attached"),
        "no fact was appended after the end"
    );
}

/// Discover heuristically, confirm, then watch the runtime exit. The Run whose
/// start was withheld keeps its end out of the log too — confirming an
/// association later never promotes earlier heuristic runtime metadata into
/// durable truth, and the store keeps no record of a withheld Run to change
/// its mind about.
#[test]
fn confirming_an_association_does_not_make_an_earlier_run_recordable() {
    let mut store = TestStore::new("withheld-then-confirmed");
    let node = store.node();
    let SessionResolution::Created { session, binding } = store
        .resolve_or_create_session(
            key(node, BindingKind::Runtime, "pid-77"),
            Provenance::Discovered,
            suspected_runtime(),
            instant(10),
        )
        .expect("resolved")
    else {
        panic!("a new external identity is a new Session");
    };
    let withheld = store
        .record_run_started(
            binding.id(),
            EvidenceSource::NodeRuntimeObservation,
            OccurrenceTime::Unknown,
        )
        .expect("a Run exists");
    store
        .confirm_binding(
            binding.id(),
            evidence(EvidenceSource::ProviderHook, Assurance::Attested),
        )
        .expect("confirmed");

    let ended = store
        .record_run_ended(
            withheld.run(),
            RunEnd::Exited(ExitCause::Completed),
            OccurrenceTime::Authoritative(instant(50)),
        )
        .expect("an ordinary sequence, not a refusal");

    assert_eq!(ended, Durability::Withheld);
    assert_eq!(store.runs_of(session.id()).expect("readable"), Vec::new());
}

/// A Run withheld while its association was heuristic keeps its identity, so
/// once the association is established its facts can be appended — the
/// sequence D6 describes, gated on the runtime evidence still supporting them.
#[test]
fn a_withheld_run_becomes_durable_under_its_own_identity() {
    let mut store = TestStore::new("backfill");
    let node = store.node();
    let SessionResolution::Created { session, binding } = store
        .resolve_or_create_session(
            key(node, BindingKind::Runtime, "pid-77"),
            Provenance::Discovered,
            suspected_runtime(),
            instant(10),
        )
        .expect("resolved")
    else {
        panic!("a new external identity is a new Session");
    };
    let withheld = store
        .record_run_started(
            binding.id(),
            EvidenceSource::NodeRuntimeObservation,
            OccurrenceTime::Authoritative(instant(20)),
        )
        .expect("a Run exists");
    assert_eq!(withheld.durability(), Durability::Withheld);

    store
        .confirm_binding(
            binding.id(),
            evidence(EvidenceSource::ProviderHook, Assurance::Attested),
        )
        .expect("confirmed");
    let recorded = store
        .record_withheld_run_started(
            withheld.run(),
            EvidenceSource::NodeRuntimeObservation,
            OccurrenceTime::Authoritative(instant(20)),
        )
        .expect("appended now");

    assert_eq!(recorded.durability(), Durability::Recorded);
    assert_eq!(recorded.run().id(), withheld.run().id());
    let runs = store.runs_of(session.id()).expect("readable");
    assert_eq!(runs.len(), 1);
    assert_eq!(
        runs[0].started_at(),
        OccurrenceTime::Authoritative(instant(20)),
        "the occurrence the runtime supports, not the moment it was accepted"
    );
    assert_eq!(
        kinds(&store.events_of(session.id()).expect("readable")),
        [
            "session-created",
            "binding-added",
            "binding-confirmed",
            "run-started"
        ],
        "appended after the confirmation, never inserted before it"
    );
    assert_eq!(
        store
            .record_run_ended(
                recorded.run(),
                RunEnd::Exited(ExitCause::Completed),
                OccurrenceTime::Authoritative(instant(30)),
            )
            .expect("recorded"),
        Durability::Recorded
    );
}

/// A Run the log already holds has no start still waiting to be appended.
#[test]
fn a_recorded_run_cannot_have_its_start_appended_again() {
    let mut store = TestStore::new("backfill-twice");
    let (_, binding) = managed_session(&mut store, "run-a");
    let run = store
        .record_run_started(
            binding,
            EvidenceSource::CorralConstructed,
            OccurrenceTime::Authoritative(instant(20)),
        )
        .expect("recorded");

    let refusal = store
        .record_withheld_run_started(
            run.run(),
            EvidenceSource::CorralConstructed,
            OccurrenceTime::Authoritative(instant(21)),
        )
        .expect_err("refused");

    assert!(matches!(
        refusal,
        StateError::Refused(Refusal::RunAlreadyRecorded(_))
    ));
}

/// Confirmation strengthens. Persisting a weakening would be the
/// assurance-change write Q15 deferred, and an append-only log could never
/// take it back.
#[test]
fn a_confirmation_may_not_weaken_a_binding() {
    let mut store = TestStore::new("weakening");
    let (session, binding) = managed_session(&mut store, "run-a");

    let refusal = store
        .confirm_binding(
            binding,
            evidence(EvidenceSource::Correlation, Assurance::Heuristic),
        )
        .expect_err("refused");

    assert!(matches!(
        refusal,
        StateError::Refused(Refusal::UnsupportedConfirmation { .. })
    ));
    assert_eq!(
        store.bindings_of(session).expect("readable")[0].assurance(),
        Assurance::Deterministic
    );
}

/// Corral links and unlinks, never merges. A caller that names a Session is
/// told when the identity belongs to another one, rather than handed somebody
/// else's binding as though the link had happened.
#[test]
fn an_identity_another_session_holds_is_not_this_callers_binding() {
    let mut store = TestStore::new("claimed");
    let node = store.node();
    let (owner, _) = managed_session(&mut store, "run-a");
    let (other, _) = managed_session(&mut store, "run-b");
    let identity = key(node, BindingKind::ProviderSession, "sess-1");
    store
        .bind(
            owner,
            identity.clone(),
            Provenance::Discovered,
            evidence(EvidenceSource::ProviderHook, Assurance::Attested),
            instant(20),
        )
        .expect("bound");

    let refusal = store
        .bind(
            other,
            identity,
            Provenance::Discovered,
            evidence(EvidenceSource::ProviderHook, Assurance::Attested),
            instant(21),
        )
        .expect_err("refused");

    assert!(matches!(
        refusal,
        StateError::Refused(Refusal::BindingClaimedByAnotherSession { session, .. })
            if session == owner
    ));
}

/// D8 keeps an edge whose target is deleted later. An edge that never had a
/// target is a producer bug, and the log cannot take it back.
#[test]
fn lineage_naming_a_parent_the_store_does_not_hold_is_refused() {
    let mut store = TestStore::new("ghost-parent");
    let (child, _) = managed_session(&mut store, "run-a");
    let ghost = CorralSessionId::mint();

    let refusal = store
        .record_fork(SessionLineage::record(child, ghost, Assurance::Deterministic).expect("ok"))
        .expect_err("refused");

    assert!(matches!(
        refusal,
        StateError::Refused(Refusal::UnknownSession(named)) if named == ghost
    ));
    assert_eq!(store.lineage_of(child).expect("readable"), None);
}

/// A perfectly good database that is not Corral's registry is not an unwritten
/// one: creating the registry inside it would put the authoritative store and
/// something unrelated in one file.
#[test]
fn a_database_that_is_not_the_registry_is_refused() {
    let store = TestStore::new("foreign");
    let path = store.directory.join("someone-elses.sqlite3");
    #[allow(clippy::disallowed_methods)]
    let foreign = rusqlite::Connection::open(&path).expect("open");
    foreign
        .execute_batch("CREATE TABLE notes (id INTEGER PRIMARY KEY)")
        .expect("write a foreign table");
    drop(foreign);

    let Err(error) = Store::open(&path) else {
        panic!("a foreign database is not an empty registry");
    };

    assert!(matches!(
        error,
        StateError::Fatal(FatalState::Unopenable { .. })
    ));
}

/// A Run's position is where its episode sits, not where its acceptance fell.
/// A start learned late still takes the place its occurrence earns.
#[test]
fn a_backfilled_run_takes_the_position_its_occurrence_earns() {
    let mut store = TestStore::new("position");
    let node = store.node();
    let SessionResolution::Created { session, binding } = store
        .resolve_or_create_session(
            key(node, BindingKind::Runtime, "pid-77"),
            Provenance::Discovered,
            suspected_runtime(),
            instant(10),
        )
        .expect("resolved")
    else {
        panic!("a new external identity is a new Session");
    };
    let early = store
        .record_run_started(
            binding.id(),
            EvidenceSource::NodeRuntimeObservation,
            OccurrenceTime::Authoritative(instant(20)),
        )
        .expect("a Run exists");
    store
        .confirm_binding(
            binding.id(),
            evidence(EvidenceSource::ProviderHook, Assurance::Attested),
        )
        .expect("confirmed");
    let late = store
        .record_run_started(
            binding.id(),
            EvidenceSource::NodeRuntimeObservation,
            OccurrenceTime::Authoritative(instant(40)),
        )
        .expect("recorded");
    store
        .record_run_ended(
            late.run(),
            RunEnd::Exited(ExitCause::Completed),
            OccurrenceTime::Authoritative(instant(41)),
        )
        .expect("recorded");

    store
        .record_withheld_run_started(
            early.run(),
            EvidenceSource::NodeRuntimeObservation,
            OccurrenceTime::Authoritative(instant(20)),
        )
        .expect("appended now");

    let runs = store.runs_of(session.id()).expect("readable");
    assert_eq!(runs.len(), 2);
    assert_eq!(
        runs[0].id(),
        early.run().id(),
        "the earlier episode is first"
    );
    assert_eq!(runs[0].ordinal(), Some(RunOrdinal::FIRST));
    assert_eq!(runs[1].id(), late.run().id());
    assert_eq!(runs[1].ordinal(), Some(RunOrdinal::from_position(2)));
}

/// The store kept no record of a withheld Run, so the Run itself is what it
/// checks: one that names a Session its binding does not is not this
/// association's Run.
#[test]
fn a_withheld_run_cannot_be_filed_under_an_association_it_does_not_claim() {
    let mut store = TestStore::new("misfiled");
    let (_, binding) = managed_session(&mut store, "run-a");
    let stranger = CorralSessionId::mint();
    let misfiled = Run::started(
        RunId::mint(),
        stranger,
        binding,
        OccurrenceTime::Authoritative(instant(20)),
    );

    let refusal = store
        .record_withheld_run_started(
            &misfiled,
            EvidenceSource::CorralConstructed,
            OccurrenceTime::Authoritative(instant(20)),
        )
        .expect_err("refused");

    assert!(matches!(
        refusal,
        StateError::Refused(Refusal::UnknownSession(named)) if named == stranger
    ));
}

/// Confirming a binding the store never held is an unknown binding, not weak
/// evidence — the refusal has to name the real problem.
#[test]
fn confirming_a_binding_the_store_never_held_says_so() {
    let mut store = TestStore::new("confirm-unknown");
    let absent = BindingId::mint();

    let refusal = store
        .confirm_binding(
            absent,
            evidence(EvidenceSource::Correlation, Assurance::Heuristic),
        )
        .expect_err("refused");

    assert!(matches!(
        refusal,
        StateError::Refused(Refusal::UnknownBinding(named)) if named == absent
    ));
}

/// A registry that lost its version row is a damaged registry, not somebody
/// else's database. On a fail-closed startup path the message is all the
/// operator has, and it must point at the right file.
#[test]
fn a_registry_that_lost_its_version_is_not_mistaken_for_a_stranger() {
    let store = TestStore::new("version-dropped");
    store.behind_the_daemons_back("DROP TABLE schema_version");
    let path = store.path();

    let Err(error) = Store::open(&path) else {
        panic!("a registry without its version is not usable");
    };

    assert!(
        matches!(
            error,
            StateError::Fatal(FatalState::SchemaVersionMismatch { found: None, .. })
        ),
        "expected a damaged registry, got {error}"
    );
}

/// A lineage cycle can never be removed from an append-only log with no
/// correction event, and every consumer walking ancestry would have to invent
/// its own depth cap.
#[test]
fn lineage_that_would_close_a_loop_is_refused() {
    let mut store = TestStore::new("cycle");
    let (a, _) = managed_session(&mut store, "run-a");
    let (b, _) = managed_session(&mut store, "run-b");
    let (c, _) = managed_session(&mut store, "run-c");
    store
        .record_fork(SessionLineage::record(b, a, Assurance::Deterministic).expect("recordable"))
        .expect("recorded");
    store
        .record_fork(SessionLineage::record(c, b, Assurance::Deterministic).expect("recordable"))
        .expect("recorded");

    let refusal = store
        .record_fork(SessionLineage::record(a, c, Assurance::Deterministic).expect("recordable"))
        .expect_err("refused");

    assert!(matches!(
        refusal,
        StateError::Refused(Refusal::LineageWouldCycle { .. })
    ));
    assert_eq!(store.lineage_of(a).expect("readable"), None);
}

/// The store's own foreign keys reject a Session that is not there, and the
/// refusal says which Session rather than handing back an engine message.
#[test]
fn naming_a_session_the_store_does_not_hold_is_refused_by_name() {
    let mut store = TestStore::new("unknown-session");
    let node = store.node();
    let ghost = CorralSessionId::mint();

    let refusal = store
        .bind(
            ghost,
            key(node, BindingKind::ProviderSession, "sess-9"),
            Provenance::Discovered,
            evidence(EvidenceSource::ProviderHook, Assurance::Attested),
            instant(10),
        )
        .expect_err("refused");

    assert!(matches!(
        refusal,
        StateError::Refused(Refusal::UnknownSession(named)) if named == ghost
    ));
}

#[test]
fn a_first_execution_mutates_once_and_stores_a_receipt() {
    let mut store = TestStore::new("receipt-first");
    let command = command("cmd-1", "/work");

    let accepted = store
        .create_session(&command, instant(10))
        .expect("created");

    assert!(matches!(accepted, CommandAcceptance::Executed(_)));
    assert_eq!(store.sessions().expect("readable").len(), 1);
    assert_eq!(
        store.receipt(command.id()).expect("readable").as_ref(),
        Some(accepted.receipt())
    );
}

#[test]
fn the_same_semantic_command_returns_the_original_receipt() {
    let mut store = TestStore::new("receipt-replay");
    let command = command("cmd-1", "/work");
    let first = store
        .create_session(&command, instant(10))
        .expect("created");

    let again = store
        .create_session(&command, instant(99))
        .expect("replayed");

    assert!(matches!(again, CommandAcceptance::Replayed(_)));
    assert_eq!(again.receipt(), first.receipt());
    assert_eq!(
        store.sessions().expect("readable").len(),
        1,
        "a retry mutates nothing a second time"
    );
}

/// One command id means one immutable semantic command, for the life of the
/// node's durable state.
#[test]
fn the_same_id_with_a_different_command_conflicts_and_changes_nothing() {
    let mut store = TestStore::new("receipt-conflict");
    let first = store
        .create_session(&command("cmd-1", "/work"), instant(10))
        .expect("created");

    let refusal = store
        .create_session(&command("cmd-1", "/elsewhere"), instant(20))
        .expect_err("refused");

    assert!(matches!(
        refusal,
        StateError::Refused(Refusal::CommandIdConflict { .. })
    ));
    assert_eq!(store.sessions().expect("readable").len(), 1);
    assert_eq!(
        store
            .receipt(&CommandId::new("cmd-1").expect("usable"))
            .expect("readable")
            .as_ref(),
        Some(first.receipt()),
        "the original receipt is untouched"
    );
}

/// Idempotency binds to what a command means. Two descriptions of one command
/// that differ only in how they were written are the same command.
#[test]
fn equivalent_descriptions_of_one_command_do_not_conflict() {
    let mut store = TestStore::new("receipt-equivalent");
    let kind = || CommandKind::new("session.create").expect("usable");
    let one = Command::new(
        CommandId::new("cmd-1").expect("usable"),
        CommandFingerprint::builder(kind())
            .input("cwd", "/work")
            .input("provider", "claude-code")
            .build(),
    );
    let other = Command::new(
        CommandId::new("cmd-1").expect("usable"),
        CommandFingerprint::builder(kind())
            .input("provider", "claude-code")
            .input("cwd", "/work")
            .build(),
    );
    let first = store.create_session(&one, instant(10)).expect("created");

    let again = store.create_session(&other, instant(20)).expect("replayed");

    assert!(matches!(again, CommandAcceptance::Replayed(_)));
    assert_eq!(again.receipt(), first.receipt());
}

/// A command id is unique in the node's durable command namespace, and a
/// daemon restart does not reset it — otherwise the next daemon would execute
/// a command the last one already performed.
#[test]
fn a_command_id_stays_taken_across_a_restart() {
    let mut store = TestStore::new("receipt-restart");
    let command = command("cmd-1", "/work");
    let first = store
        .create_session(&command, instant(10))
        .expect("created");

    store.reopen();
    let again = store
        .create_session(&command, instant(20))
        .expect("replayed");

    assert_eq!(again.receipt(), first.receipt());
    assert_eq!(store.sessions().expect("readable").len(), 1);
}

/// The log owns durable truth and the projections only summarize it: clearing
/// them and replaying reproduces exactly what was there. A projection field
/// that survived this by not being derivable would be an architecture
/// violation, not a bug to patch.
#[test]
fn replaying_the_log_reproduces_the_projections() {
    let mut store = TestStore::new("replay");
    let commands = every_durable_transition(&mut store);
    let before = snapshot(&mut store, &commands);

    store.rebuild_projections().expect("rebuilt");

    assert_eq!(snapshot(&mut store, &commands), before);
}

/// Every durable transition PR2 can produce, so the replay above covers the
/// whole accepted event set rather than a convenient corner of it.
fn every_durable_transition(store: &mut Store) -> Vec<CommandId> {
    let node = store.node();
    let created = command("cmd-managed", "/work");
    let accepted = store
        .create_session(&created, instant(10))
        .expect("created");
    let CommandOutcome::SessionCreated(managed) = accepted.receipt().outcome();
    let BindingResolution::Created(runtime) = store
        .bind(
            managed,
            key(node, BindingKind::Runtime, "run-a"),
            Provenance::CorralCreated,
            owned_runtime(),
            instant(11),
        )
        .expect("bound")
    else {
        panic!("a new external identity is a new binding");
    };

    let run = store
        .record_run_started(
            runtime.id(),
            EvidenceSource::CorralConstructed,
            OccurrenceTime::Authoritative(instant(12)),
        )
        .expect("recorded");
    store
        .record_run_attached(run.run(), instant(13))
        .expect("recorded");
    store
        .record_run_detached(run.run(), instant(14))
        .expect("recorded");
    store
        .record_run_ended(
            run.run(),
            RunEnd::Exited(ExitCause::Terminated),
            OccurrenceTime::Authoritative(instant(15)),
        )
        .expect("recorded");
    store
        .record_run_started(
            runtime.id(),
            EvidenceSource::CorralConstructed,
            OccurrenceTime::Unknown,
        )
        .expect("recorded");

    let SessionResolution::Created {
        session: observed,
        binding: provider,
    } = store
        .resolve_or_create_session(
            key(node, BindingKind::ProviderSession, "sess-1"),
            Provenance::Discovered,
            evidence(EvidenceSource::Correlation, Assurance::Heuristic),
            instant(20),
        )
        .expect("resolved")
    else {
        panic!("a new external identity is a new Session");
    };
    store
        .confirm_binding(
            provider.id(),
            evidence(EvidenceSource::ProviderHook, Assurance::Attested),
        )
        .expect("confirmed");
    store
        .record_fork(
            SessionLineage::record(observed.id(), managed, Assurance::Deterministic)
                .expect("recordable"),
        )
        .expect("recorded");

    vec![created.id().clone()]
}

/// Everything the projections hold, rendered so two states can be compared.
fn snapshot(store: &mut Store, commands: &[CommandId]) -> String {
    let mut rendered = String::new();
    for session in store.sessions().expect("readable") {
        writeln!(
            rendered,
            "session {} {:?}",
            session.id(),
            session.created_at()
        )
        .expect("render");
        for binding in store.bindings_of(session.id()).expect("readable") {
            writeln!(
                rendered,
                "  binding {} {:?} {:?} {:?} {:?}",
                binding.id(),
                binding.kind(),
                binding.provenance(),
                binding.evidence(),
                binding.key().external_id(),
            )
            .expect("render");
        }
        for run in store.runs_of(session.id()).expect("readable") {
            writeln!(
                rendered,
                "  run {} {:?} {:?} {:?} {:?}",
                run.id(),
                run.ordinal(),
                run.started_at(),
                run.end(),
                run.ended_at(),
            )
            .expect("render");
        }
        if let Some(edge) = store.lineage_of(session.id()).expect("readable") {
            writeln!(
                rendered,
                "  parent {} {:?}",
                edge.parent(),
                edge.assurance()
            )
            .expect("render");
        }
    }
    for command in commands {
        writeln!(
            rendered,
            "receipt {:?}",
            store.receipt(command).expect("readable")
        )
        .expect("render");
    }
    rendered
}

/// A runtime binding names one runtime, and one runtime runs one episode at a
/// time. Two live Runs behind one binding would leave PR8 choosing between
/// contradictory episodes for one row.
#[test]
fn one_runtime_binding_runs_one_episode_at_a_time() {
    let mut store = TestStore::new("one-episode");
    let (session, binding) = managed_session(&mut store, "run-a");
    let first = store
        .record_run_started(
            binding,
            EvidenceSource::CorralConstructed,
            OccurrenceTime::Authoritative(instant(20)),
        )
        .expect("recorded");

    let refusal = store
        .record_run_started(
            binding,
            EvidenceSource::CorralConstructed,
            OccurrenceTime::Authoritative(instant(21)),
        )
        .expect_err("refused");

    assert!(matches!(
        refusal,
        StateError::Refused(Refusal::RunAlreadyLive { run, .. }) if run == first.run().id()
    ));
    assert_eq!(store.runs_of(session).expect("readable").len(), 1);
}

/// A real clock carries nanoseconds and the store keeps milliseconds, so a
/// value it hands back has to be the value it will read back — otherwise the
/// receipt a retry gets never equals the one the first execution returned.
#[test]
fn a_receipt_survives_a_clock_finer_than_the_store() {
    let mut store = TestStore::new("precision");
    let command = command("cmd-1", "/work");
    let precise = SystemTime::UNIX_EPOCH + Duration::new(1_766_000_000, 123_456_789);

    let first = store.create_session(&command, precise).expect("created");
    let again = store
        .create_session(&command, precise + Duration::from_secs(5))
        .expect("replayed");

    assert!(matches!(again, CommandAcceptance::Replayed(_)));
    assert_eq!(again.receipt(), first.receipt());
    assert_eq!(
        store.receipt(command.id()).expect("readable").as_ref(),
        Some(first.receipt()),
        "the receipt the write returned is the receipt the store holds"
    );
}

/// The same rule for every value a write hands back, not just receipts.
#[test]
fn a_binding_is_returned_as_the_store_will_read_it_back() {
    let mut store = TestStore::new("precision-binding");
    let node = store.node();
    let precise = SystemTime::UNIX_EPOCH + Duration::new(1_766_000_000, 987_654_321);
    let SessionResolution::Created { session, binding } = store
        .resolve_or_create_session(
            key(node, BindingKind::ProviderSession, "sess-1"),
            Provenance::Discovered,
            Evidence::new(EvidenceSource::ProviderHook, Assurance::Attested, precise),
            precise,
        )
        .expect("resolved")
    else {
        panic!("a new external identity is a new Session");
    };

    assert_eq!(
        store.binding(binding.id()).expect("readable"),
        Some(binding)
    );
    assert_eq!(store.sessions().expect("readable"), vec![session]);
}

/// The canonical fingerprint is stored whole so a conflict can be read. A
/// durable row is still not a place for unbounded client input.
#[test]
fn an_oversized_fingerprint_is_refused_before_anything_is_written() {
    let mut store = TestStore::new("fingerprint-size");
    let huge = Command::new(
        CommandId::new("cmd-1").expect("usable"),
        CommandFingerprint::builder(CommandKind::new("session.create").expect("usable"))
            .input("cwd", "x".repeat(8192))
            .build(),
    );

    let refusal = store
        .create_session(&huge, instant(10))
        .expect_err("refused");

    assert!(matches!(
        refusal,
        StateError::Refused(Refusal::FingerprintTooLarge { .. })
    ));
    assert_eq!(store.sessions().expect("readable"), Vec::new());
}

/// Contention is the canonical transient condition. A store that latched it
/// would let one backup tool end the daemon.
#[test]
fn a_refusal_leaves_the_store_usable() {
    let mut store = TestStore::new("refusal-usable");
    store
        .create_session(&command("cmd-1", "/work"), instant(10))
        .expect("created");

    let refusal = store
        .create_session(&command("cmd-1", "/elsewhere"), instant(20))
        .expect_err("refused");

    assert!(!refusal.is_fatal());
    assert_eq!(
        store.sessions().expect("still readable").len(),
        1,
        "a refused write does not stop the store answering"
    );
}

/// Another writer holding the file is something to wait for, not to conclude
/// anything from. Without the wait, one concurrent writer would turn every
/// operation into a hard failure.
#[test]
fn a_store_held_by_another_writer_is_waited_for() {
    let mut store = TestStore::new("contention");
    // Being the second opener is the condition under test.
    #[allow(clippy::disallowed_methods)]
    let blocker = rusqlite::Connection::open(store.path()).expect("open");
    blocker
        .execute_batch("BEGIN EXCLUSIVE")
        .expect("hold the store");
    let released = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        blocker.execute_batch("COMMIT").expect("release the store");
    });

    let sessions = store.sessions().expect("waited rather than gave up");

    assert_eq!(sessions, Vec::new());
    released.join().expect("the other writer finished");
}

/// The conclusion has to outlive whoever reached it: a daemon reads its exit
/// status from here, and a task cancelled mid-shutdown must not be able to
/// take the answer with it.
#[test]
fn a_store_that_stopped_vouching_says_so_afterwards() {
    let mut store = TestStore::new("stopped-vouching");
    assert!(!store.stopped_vouching());
    let replacement = NodeId::mint();
    store.behind_the_daemons_back(&format!(
        "UPDATE node_identity SET node_id = '{replacement}'"
    ));

    store.sessions().expect_err("fatal");

    assert!(store.stopped_vouching());
}

/// A store that is not the schema this build knows is refused rather than
/// guessed at: no migration exists, and reading it as if it were this schema
/// would reinterpret recorded facts.
#[test]
fn a_store_at_an_unknown_schema_is_refused() {
    let mut store = TestStore::new("schema");
    store.behind_the_daemons_back("UPDATE schema_version SET version = 99");

    let error = store.sessions().expect_err("fatal");

    assert!(matches!(
        error,
        StateError::Fatal(FatalState::SchemaVersionMismatch {
            found: Some(99),
            ..
        })
    ));
}

/// A store replaced or rewritten underneath the daemon invalidates every fact
/// read since. Once the store stops vouching it never answers normally again,
/// even if the file is put back.
#[test]
fn a_store_that_stops_vouching_never_answers_normally_again() {
    let mut store = TestStore::new("poison");
    let node = store.node();
    managed_session(&mut store, "run-a");
    let replacement = NodeId::mint();
    store.behind_the_daemons_back(&format!(
        "UPDATE node_identity SET node_id = '{replacement}'"
    ));

    let error = store.sessions().expect_err("fatal");
    store.behind_the_daemons_back(&format!("UPDATE node_identity SET node_id = '{node}'"));
    let after_repair = store.sessions().expect_err("still fatal");

    assert!(matches!(
        error,
        StateError::Fatal(FatalState::StoreIdentityChanged { .. })
    ));
    assert!(after_repair.is_fatal());
}

/// A file that is not a Corral store at all is a startup failure, not an empty
/// registry.
#[test]
fn a_file_that_is_not_a_store_is_refused() {
    let store = TestStore::new("garbage");
    let path = store.directory.join("not-a-store.sqlite3");
    std::fs::write(&path, b"this is not a database").expect("write");

    let Err(error) = Store::open(&path) else {
        panic!("a file that is not a database is not a store");
    };

    assert!(matches!(
        error,
        StateError::Fatal(FatalState::Unopenable { .. })
    ));
}

#[test]
fn a_store_in_a_directory_that_does_not_exist_is_refused() {
    let store = TestStore::new("missing-dir");
    let path = store.directory.join("absent").join(FILE);

    let Err(error) = Store::open(&path) else {
        panic!("a store cannot be created where its directory is not");
    };

    assert!(matches!(
        error,
        StateError::Fatal(FatalState::Unopenable { .. })
    ));
}
