use std::fmt::Write as _;
use std::ops::{Deref, DerefMut};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use corral_core::{
    Assurance, BindingKind, CommandFingerprint, CommandId, CommandKind, ConfigTarget,
    ControlEligibility, EvidenceSource, ExitCause, ExternalId, IntegrationIntent, ProviderId,
    RepairAuthority, RepairBudget, RepairFingerprint, RepairableDrift, RunOrdinal,
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

/// The key of a runtime Corral created itself.
///
/// The external id is named here rather than minted, so a test can talk about
/// the same managed binding twice. Its provider is the reserved one, because
/// that is what a `CorralCreated` runtime binding must carry (ADR 0008 D3).
fn managed_key(node: NodeId, external: &str) -> BindingKey {
    BindingKey::new(
        node,
        BindingKind::Runtime,
        ProviderId::corral(),
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

/// Open a managed session under a command, the way `session.new` does.
fn opened(
    store: &mut Store,
    command: &Command,
    at: SystemTime,
) -> Result<StartedManagedSession, StateError> {
    store.start_managed_session(
        command,
        CorralSessionId::mint(),
        RunId::mint(),
        OccurrenceTime::Authoritative(at),
        at,
    )
}

fn kinds(events: &[RecordedEvent]) -> Vec<&'static str> {
    events
        .iter()
        .map(|recorded| recorded.event().kind())
        .collect()
}

/// A Session with the managed runtime binding Corral owns for it, and no Run
/// yet.
///
/// Deliberately not `start_managed_session`: the Run tests below need a
/// binding with nothing running under it, and one that already had a live Run
/// would refuse the second episode they are about to open.
fn managed_session(store: &mut Store, external: &str) -> (CorralSessionId, BindingId) {
    let node = store.node();
    let SessionResolution::Created { session, binding } = store
        .resolve_or_create_session(
            managed_key(node, external),
            Provenance::CorralCreated,
            owned_runtime(),
            instant(10),
        )
        .expect("resolved")
    else {
        panic!("a new external identity is a new Session");
    };
    (session.id(), binding.id())
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
/// The same provider session, met a second time under a different kind of
/// binding, is the same Session.
///
/// Enumeration files a history binding for an id the store named; the hook
/// then reports that id live and files a provider-session binding for it.
/// Both name one conversation, so resolution looks the identity up across
/// kinds and the provider-session binding wins where several name it — the
/// alternative is two rows for one agent, which is the duplication a binding
/// key exists to prevent (ADR 0016 D2).
#[test]
fn one_provider_session_met_under_two_binding_kinds_is_one_session() {
    let mut store = TestStore::new("cross-kind");
    let node = store.node();

    let SessionResolution::Created { session: known, .. } = store
        .resolve_or_create_session(
            key(node, BindingKind::History, "sess-x"),
            Provenance::Discovered,
            evidence(EvidenceSource::HistoryRecord, Assurance::Attested),
            instant(10),
        )
        .expect("resolved")
    else {
        panic!("an identity nothing claims is a new Session");
    };

    let discovered = store
        .resolve_or_create_session(
            key(node, BindingKind::ProviderSession, "sess-x"),
            Provenance::Discovered,
            evidence(EvidenceSource::ProviderHook, Assurance::Attested),
            instant(20),
        )
        .expect("resolved");

    let (SessionResolution::Created { session, .. } | SessionResolution::Existing { session, .. }) =
        discovered;
    assert_eq!(
        session.id(),
        known.id(),
        "discovery minted a second Session for a provider session the store already knew"
    );
}

/// Binding a conversation another Session already holds is refused, even
/// when the key itself is free.
///
/// A history row names a provider session; a Corral-launched agent then
/// resumes that same conversation from inside itself and its hook reports the
/// id. The provider-session key is unused, so the exact-key check passes —
/// and adding the edge would put one agent behind two rows, which is what
/// binding uniqueness exists to stop (`ARCHITECTURE.md` §1).
#[test]
fn a_conversation_another_session_holds_is_not_bound_to_a_second_one() {
    let mut store = TestStore::new("cross-kind-bind");
    let node = store.node();
    let SessionResolution::Created { session: known, .. } = store
        .resolve_or_create_session(
            key(node, BindingKind::History, "sess-y"),
            Provenance::Discovered,
            evidence(EvidenceSource::HistoryRecord, Assurance::Attested),
            instant(10),
        )
        .expect("resolved")
    else {
        panic!("an identity nothing claims is a new Session");
    };
    let (other, _) = managed_session(&mut store, "run-elsewhere");

    let refused = store.bind(
        other,
        key(node, BindingKind::ProviderSession, "sess-y"),
        Provenance::Discovered,
        evidence(EvidenceSource::ProviderHook, Assurance::Attested),
        instant(20),
    );

    assert!(
        matches!(
            refused,
            Err(StateError::Refused(Refusal::BindingClaimedByAnotherSession {
                session,
                ..
            })) if session == known.id()
        ),
        "{refused:?}"
    );
}

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
            managed_key(node, "run-b"),
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

/// A Session has one provider identity, and the store is what enforces it.
///
/// Uniqueness on the key answers the other direction — one identity reaching
/// two Sessions. Nothing but this answers a Session reaching two identities,
/// which is the state ADR 0004 D8 calls a contest rather than a second fact.
#[test]
fn a_session_holds_at_most_one_provider_session_binding() {
    let mut store = TestStore::new("one-identity");
    let node = store.node();
    let (session, _) = managed_session(&mut store, "run-a");
    let BindingResolution::Created(first) = store
        .bind(
            session,
            key(node, BindingKind::ProviderSession, "sess-1"),
            Provenance::CorralCreated,
            evidence(EvidenceSource::ProviderHook, Assurance::Attested),
            instant(12),
        )
        .expect("bound")
    else {
        panic!("a new external identity is a new binding");
    };

    let refusal = store
        .bind(
            session,
            key(node, BindingKind::ProviderSession, "sess-2"),
            Provenance::CorralCreated,
            evidence(EvidenceSource::ProviderHook, Assurance::Attested),
            instant(13),
        )
        .expect_err("refused");

    assert!(
        matches!(
            refusal,
            StateError::Refused(Refusal::ProviderSessionBindingExists { existing, .. })
                if existing == first.id()
        ),
        "{refusal}"
    );
    // Re-offering the identity it already holds is not a second one.
    assert!(
        store
            .bind(
                session,
                key(node, BindingKind::ProviderSession, "sess-1"),
                Provenance::CorralCreated,
                evidence(EvidenceSource::ProviderHook, Assurance::Attested),
                instant(14),
            )
            .is_ok()
    );
}

/// Confirming a second runtime binding acquires control just as adding one
/// does, so it meets the same rule.
///
/// The candidate is one the user linked: provenance is what says whether
/// Corral may drive a runtime at all, so a *discovered* one could never
/// acquire control however strong its evidence became, and confirming it
/// would prove nothing about this rule (ADR 0014 D6).
#[test]
fn confirming_a_second_runtime_binding_is_refused() {
    let mut store = TestStore::new("confirm-second");
    let node = store.node();
    let (session, _) = managed_session(&mut store, "run-a");
    let BindingResolution::Created(weak) = store
        .bind(
            session,
            key(node, BindingKind::Runtime, "run-b"),
            Provenance::UserLinked,
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
            RunId::mint(),
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
            RunId::mint(),
            binding.id(),
            EvidenceSource::NodeRuntimeObservation,
            OccurrenceTime::Unknown,
        )
        .expect("a Run exists");
    let ended = store
        .record_run_ended(
            recorded.run().id(),
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
            RunId::mint(),
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
            RunId::mint(),
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
            RunId::mint(),
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
            RunId::mint(),
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
            RunId::mint(),
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
            RunId::mint(),
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
            RunId::mint(),
            binding,
            EvidenceSource::CorralConstructed,
            OccurrenceTime::Authoritative(instant(20)),
        )
        .expect("recorded");
    store
        .record_run_ended(
            first.run().id(),
            RunEnd::Exited(ExitCause::Completed),
            OccurrenceTime::Authoritative(instant(30)),
        )
        .expect("recorded");

    let second = store
        .record_run_started(
            RunId::mint(),
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
            RunId::mint(),
            binding,
            EvidenceSource::CorralConstructed,
            OccurrenceTime::Authoritative(instant(20)),
        )
        .expect("recorded");
    store
        .record_run_ended(
            run.run().id(),
            RunEnd::Exited(ExitCause::Completed),
            OccurrenceTime::Authoritative(instant(30)),
        )
        .expect("recorded");

    let refusal = store
        .record_run_ended(
            run.run().id(),
            RunEnd::Unverifiable,
            OccurrenceTime::Unknown,
        )
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
            RunId::mint(),
            binding,
            EvidenceSource::CorralConstructed,
            OccurrenceTime::Authoritative(instant(20)),
        )
        .expect("recorded");

    store
        .record_run_attached(run.run().id(), instant(21))
        .expect("recorded");
    store
        .record_run_detached(run.run().id(), instant(22))
        .expect("recorded");

    let runs = store.runs_of(session).expect("readable");
    assert_eq!(runs.len(), 1);
    assert!(runs[0].is_live(), "detaching is not an end");
    assert_eq!(
        kinds(&store.events_of(session).expect("readable")),
        [
            "session-created",
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
            RunId::mint(),
            binding,
            EvidenceSource::CorralConstructed,
            OccurrenceTime::Authoritative(instant(20)),
        )
        .expect("recorded");
    store
        .record_run_ended(
            run.run().id(),
            RunEnd::Exited(ExitCause::Completed),
            OccurrenceTime::Authoritative(instant(30)),
        )
        .expect("recorded");

    let refusal = store
        .record_run_attached(run.run().id(), instant(31))
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
            RunId::mint(),
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
            withheld.run().id(),
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
            RunId::mint(),
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
        .record_withheld_run_started(withheld.run(), EvidenceSource::NodeRuntimeObservation)
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
                recorded.run().id(),
                RunEnd::Exited(ExitCause::Completed),
                OccurrenceTime::Authoritative(instant(30)),
            )
            .expect("recorded"),
        Durability::Recorded
    );
}

/// Q10's own example: the Run started and ended while the binding was still
/// heuristic, and the association was confirmed afterwards — by which time a
/// new episode is normally running. Both of its facts append together, and the
/// live episode does not block a past one.
#[test]
fn a_withheld_run_that_already_ended_appends_whole() {
    let mut store = TestStore::new("ended-backfill");
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
    let past = store
        .record_run_started(
            RunId::mint(),
            binding.id(),
            EvidenceSource::NodeRuntimeObservation,
            OccurrenceTime::Authoritative(instant(20)),
        )
        .expect("a Run exists");
    let past = past.run().clone().ended(
        RunEnd::Exited(ExitCause::Completed),
        OccurrenceTime::Authoritative(instant(30)),
    );
    store
        .confirm_binding(
            binding.id(),
            evidence(EvidenceSource::ProviderHook, Assurance::Attested),
        )
        .expect("confirmed");
    let live = store
        .record_run_started(
            RunId::mint(),
            binding.id(),
            EvidenceSource::NodeRuntimeObservation,
            OccurrenceTime::Authoritative(instant(40)),
        )
        .expect("recorded");

    let appended = store
        .record_withheld_run_started(&past, EvidenceSource::NodeRuntimeObservation)
        .expect("a past episode is not blocked by the present one");

    assert_eq!(appended.durability(), Durability::Recorded);
    assert_eq!(
        appended.run().end(),
        Some(RunEnd::Exited(ExitCause::Completed)),
        "the end came back with the start"
    );
    let runs = store.runs_of(session.id()).expect("readable");
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].id(), past.id(), "the earlier episode is first");
    assert!(!runs[0].is_live(), "and it is not left running");
    assert_eq!(runs[1].id(), live.run().id());
    let stream = kinds(&store.events_of(session.id()).expect("readable"));
    assert_eq!(
        &stream[stream.len() - 2..],
        ["run-started", "run-ended"],
        "one transaction, both facts"
    );
}

/// A Run the log already holds has no start still waiting to be appended.
#[test]
fn a_recorded_run_cannot_have_its_start_appended_again() {
    let mut store = TestStore::new("backfill-twice");
    let (_, binding) = managed_session(&mut store, "run-a");
    let run = store
        .record_run_started(
            RunId::mint(),
            binding,
            EvidenceSource::CorralConstructed,
            OccurrenceTime::Authoritative(instant(20)),
        )
        .expect("recorded");

    let refusal = store
        .record_withheld_run_started(run.run(), EvidenceSource::CorralConstructed)
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
            RunId::mint(),
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
            RunId::mint(),
            binding.id(),
            EvidenceSource::NodeRuntimeObservation,
            OccurrenceTime::Authoritative(instant(40)),
        )
        .expect("recorded");
    store
        .record_run_ended(
            late.run().id(),
            RunEnd::Exited(ExitCause::Completed),
            OccurrenceTime::Authoritative(instant(41)),
        )
        .expect("recorded");

    store
        .record_withheld_run_started(early.run(), EvidenceSource::NodeRuntimeObservation)
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
        .record_withheld_run_started(&misfiled, EvidenceSource::CorralConstructed)
        .expect_err("refused");

    assert!(matches!(
        refusal,
        StateError::Refused(Refusal::RunClaimsAnotherSession { claimed, .. })
            if claimed == stranger
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

    let accepted = opened(&mut store, &command, instant(10)).expect("created");

    assert!(accepted.executed());
    assert_eq!(store.sessions().expect("readable").len(), 1);
    assert_eq!(
        store.receipt(command.id()).expect("readable").as_ref(),
        Some(accepted.acceptance().receipt())
    );
    assert_eq!(
        store.runs_of(accepted.session()).expect("readable").len(),
        1,
        "a managed session's first Run lands with the receipt that made it"
    );
}

#[test]
fn the_same_semantic_command_returns_the_original_receipt() {
    let mut store = TestStore::new("receipt-replay");
    let command = command("cmd-1", "/work");
    let first = opened(&mut store, &command, instant(10)).expect("created");

    let again = opened(&mut store, &command, instant(99)).expect("replayed");

    assert!(!again.executed());
    assert_eq!(again.acceptance().receipt(), first.acceptance().receipt());
    assert_eq!(again.session(), first.session());
    assert_eq!(
        again.run(),
        first.run(),
        "a replay names the Run the first execution made, not the one it minted"
    );
    assert_eq!(
        store.sessions().expect("readable").len(),
        1,
        "a retry mutates nothing a second time"
    );
    assert_eq!(store.runs_of(first.session()).expect("readable").len(), 1);
}

/// One command id means one immutable semantic command, for the life of the
/// node's durable state.
#[test]
fn the_same_id_with_a_different_command_conflicts_and_changes_nothing() {
    let mut store = TestStore::new("receipt-conflict");
    let first = opened(&mut store, &command("cmd-1", "/work"), instant(10)).expect("created");

    let refusal =
        opened(&mut store, &command("cmd-1", "/elsewhere"), instant(20)).expect_err("refused");

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
        Some(first.acceptance().receipt()),
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
    let first = opened(&mut store, &one, instant(10)).expect("created");

    let again = opened(&mut store, &other, instant(20)).expect("replayed");

    assert!(!again.executed());
    assert_eq!(again.acceptance().receipt(), first.acceptance().receipt());
}

/// A command id is unique in the node's durable command namespace, and a
/// daemon restart does not reset it — otherwise the next daemon would execute
/// a command the last one already performed.
#[test]
fn a_command_id_stays_taken_across_a_restart() {
    let mut store = TestStore::new("receipt-restart");
    let command = command("cmd-1", "/work");
    let first = opened(&mut store, &command, instant(10)).expect("created");

    store.reopen();
    let again = opened(&mut store, &command, instant(20)).expect("replayed");

    assert_eq!(again.acceptance().receipt(), first.acceptance().receipt());
    assert_eq!(again.run(), first.run());
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
    // Session, managed binding, first Run and receipt in one accepted command,
    // exactly as `session.new` produces them.
    let accepted = opened(store, &created, instant(10)).expect("created");
    let managed = accepted.session();
    let runtime = store
        .bindings_of(managed)
        .expect("readable")
        .into_iter()
        .next()
        .expect("the managed runtime binding");

    store
        .record_run_attached(accepted.run(), instant(13))
        .expect("recorded");
    store
        .record_run_detached(accepted.run(), instant(14))
        .expect("recorded");
    store
        .record_run_ended(
            accepted.run(),
            RunEnd::Exited(ExitCause::Terminated),
            OccurrenceTime::Authoritative(instant(15)),
        )
        .expect("recorded");
    store
        .record_run_started(
            RunId::mint(),
            runtime.id(),
            EvidenceSource::CorralConstructed,
            OccurrenceTime::Unknown,
        )
        .expect("recorded");

    // A Session whose read order differs from its acceptance order, so the
    // replay above proves the position rule and not just the row count.
    let SessionResolution::Created {
        binding: suspected, ..
    } = store
        .resolve_or_create_session(
            key(node, BindingKind::Runtime, "pid-77"),
            Provenance::Discovered,
            suspected_runtime(),
            instant(30),
        )
        .expect("resolved")
    else {
        panic!("a new external identity is a new Session");
    };
    let withheld = store
        .record_run_started(
            RunId::mint(),
            suspected.id(),
            EvidenceSource::NodeRuntimeObservation,
            OccurrenceTime::Authoritative(instant(31)),
        )
        .expect("a Run exists");
    let withheld = withheld.run().clone().ended(
        RunEnd::Exited(ExitCause::Failed),
        OccurrenceTime::Authoritative(instant(32)),
    );
    store
        .confirm_binding(
            suspected.id(),
            evidence(EvidenceSource::ProviderHook, Assurance::Attested),
        )
        .expect("confirmed");
    store
        .record_run_started(
            RunId::mint(),
            suspected.id(),
            EvidenceSource::NodeRuntimeObservation,
            OccurrenceTime::Authoritative(instant(60)),
        )
        .expect("recorded");
    store
        .record_withheld_run_started(&withheld, EvidenceSource::NodeRuntimeObservation)
        .expect("appended after the Run that started later");

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
            RunId::mint(),
            binding,
            EvidenceSource::CorralConstructed,
            OccurrenceTime::Authoritative(instant(20)),
        )
        .expect("recorded");

    let refusal = store
        .record_run_started(
            RunId::mint(),
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

    let first = opened(&mut store, &command, precise).expect("created");
    let again = opened(&mut store, &command, precise + Duration::from_secs(5)).expect("replayed");

    assert!(!again.executed());
    assert_eq!(again.acceptance().receipt(), first.acceptance().receipt());
    assert_eq!(
        store.receipt(command.id()).expect("readable").as_ref(),
        Some(first.acceptance().receipt()),
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

    let refusal = opened(&mut store, &huge, instant(10)).expect_err("refused");

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
    opened(&mut store, &command("cmd-1", "/work"), instant(10)).expect("created");

    let refusal =
        opened(&mut store, &command("cmd-1", "/elsewhere"), instant(20)).expect_err("refused");

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

/// The reserved provider namespace records who minted an identity. A runtime
/// Corral created must carry it, or its durable meaning rests on convention —
/// and the first provider phase is where conventions go (ADR 0008 D3).
#[test]
fn a_corral_created_runtime_binding_must_carry_the_reserved_provider() {
    let mut store = TestStore::new("reserved-required");
    let node = store.node();

    let refusal = store
        .resolve_or_create_session(
            key(node, BindingKind::Runtime, "run-a"),
            Provenance::CorralCreated,
            owned_runtime(),
            instant(10),
        )
        .expect_err("refused");

    assert!(matches!(
        refusal,
        StateError::Refused(Refusal::ReservedProviderNamespace {
            misuse: corral_core::ReservedNamespaceMisuse::ManagedRuntimeWithoutIt,
            ..
        })
    ));
    assert_eq!(store.sessions().expect("readable"), Vec::new());
}

/// The other direction, which is the one PR5 would otherwise break: provider
/// identity never occupies the namespace whose meaning is "Corral minted this".
#[test]
fn a_provider_binding_may_not_take_the_reserved_provider() {
    let mut store = TestStore::new("reserved-claimed");
    let node = store.node();
    let (session, _) = managed_session(&mut store, "run-a");

    let refusal = store
        .bind(
            session,
            BindingKey::new(
                node,
                BindingKind::ProviderSession,
                ProviderId::corral(),
                ExternalId::new("sess-1").expect("usable"),
            ),
            Provenance::Discovered,
            evidence(EvidenceSource::ProviderHook, Assurance::Attested),
            instant(12),
        )
        .expect_err("refused");

    assert!(matches!(
        refusal,
        StateError::Refused(Refusal::ReservedProviderNamespace {
            misuse: corral_core::ReservedNamespaceMisuse::ClaimedByAnotherIdentity,
            ..
        })
    ));
    assert_eq!(store.bindings_of(session).expect("readable").len(), 1);
}

/// A Run's id is the caller's now, so the store has to refuse one it already
/// holds — otherwise one episode could acquire two starts.
#[test]
fn a_run_id_the_log_already_holds_is_refused() {
    let mut store = TestStore::new("run-id-repeat");
    let (_, binding) = managed_session(&mut store, "run-a");
    let run = RunId::mint();
    store
        .record_run_started(
            run,
            binding,
            EvidenceSource::CorralConstructed,
            OccurrenceTime::Authoritative(instant(20)),
        )
        .expect("recorded");
    store
        .record_run_ended(
            run,
            RunEnd::Exited(ExitCause::Completed),
            OccurrenceTime::Unknown,
        )
        .expect("recorded");

    let refusal = store
        .record_run_started(
            run,
            binding,
            EvidenceSource::CorralConstructed,
            OccurrenceTime::Authoritative(instant(40)),
        )
        .expect_err("refused");

    assert!(matches!(
        refusal,
        StateError::Refused(Refusal::RunAlreadyRecorded(named)) if named == run
    ));
}

/// A daemon does not outlive its managed runtimes, so every managed episode
/// still open at startup belongs to a daemon that is gone (ADR 0007 L6).
#[test]
fn startup_closes_the_managed_episodes_a_departed_daemon_left_open() {
    let mut store = TestStore::new("reconcile");
    let command = command("cmd-1", "/work");
    let accepted = opened(&mut store, &command, instant(10)).expect("created");
    let session = accepted.session();

    store.reopen();
    let closed = store.end_unowned_managed_runs().expect("reconciled");

    assert_eq!(closed, vec![accepted.run()]);
    let runs = store.runs_of(session).expect("readable");
    assert_eq!(runs[0].end(), Some(RunEnd::Unverifiable));
    assert_eq!(
        runs[0].ended_at(),
        Some(OccurrenceTime::Unknown),
        "a daemon's startup is not when a process stopped"
    );
    assert_eq!(
        kinds(&store.events_of(session).expect("readable")),
        [
            "session-created",
            "binding-added",
            "run-started",
            "command-accepted",
            "run-ended"
        ],
        "the ending is appended, and no detach is invented for it"
    );
}

/// The predicate is ownership, never "unfinished on this node". A discovered
/// runtime is not Corral's to declare an ending for.
#[test]
fn startup_leaves_a_run_corral_does_not_manage_alone() {
    let mut store = TestStore::new("reconcile-foreign");
    let node = store.node();
    let SessionResolution::Created { binding, .. } = store
        .resolve_or_create_session(
            key(node, BindingKind::Runtime, "pid-77"),
            Provenance::Discovered,
            evidence(EvidenceSource::NodeRuntimeObservation, Assurance::Attested),
            instant(10),
        )
        .expect("resolved")
    else {
        panic!("a new external identity is a new Session");
    };
    let discovered = RunId::mint();
    store
        .record_run_started(
            discovered,
            binding.id(),
            EvidenceSource::NodeRuntimeObservation,
            OccurrenceTime::Authoritative(instant(11)),
        )
        .expect("recorded");

    let closed = store.end_unowned_managed_runs().expect("reconciled");

    assert_eq!(closed, Vec::new());
    assert!(
        store.runs_of(binding.session()).expect("readable")[0].is_live(),
        "a Run Corral did not create is not Corral's to end"
    );
}

/// Reconciliation runs on every start, and a store with nothing open must not
/// be given a fact to record.
#[test]
fn startup_records_nothing_when_no_managed_episode_is_open() {
    let mut store = TestStore::new("reconcile-empty");
    let command = command("cmd-1", "/work");
    let accepted = opened(&mut store, &command, instant(10)).expect("created");
    store
        .record_run_ended(
            accepted.run(),
            RunEnd::Exited(ExitCause::Completed),
            OccurrenceTime::Authoritative(instant(20)),
        )
        .expect("recorded");
    let before = store.events_of(accepted.session()).expect("readable").len();

    assert_eq!(
        store.end_unowned_managed_runs().expect("reconciled"),
        Vec::new()
    );
    assert_eq!(
        store.events_of(accepted.session()).expect("readable").len(),
        before
    );
}

/// A replay names the Run its command wrote, not the earliest one the Session
/// happens to hold. A later Run can carry an earlier occurrence — a clock that
/// stepped back, or a Run appended once its association was confirmed — and
/// answering with that one hands a retry a different episode than the receipt
/// describes.
#[test]
fn a_replay_names_the_run_its_command_wrote_not_the_earliest_one() {
    let mut store = TestStore::new("replay-run");
    let command = command("cmd-1", "/work");
    let first = opened(&mut store, &command, instant(500)).expect("created");
    let binding = store.bindings_of(first.session()).expect("readable")[0].id();
    store
        .record_run_ended(
            first.run(),
            RunEnd::Exited(ExitCause::Completed),
            OccurrenceTime::Authoritative(instant(600)),
        )
        .expect("recorded");
    let resumed = RunId::mint();
    store
        .record_run_started(
            resumed,
            binding,
            EvidenceSource::CorralConstructed,
            OccurrenceTime::Authoritative(instant(1)),
        )
        .expect("recorded");

    let again = opened(&mut store, &command, instant(700)).expect("replayed");

    assert_eq!(again.run(), first.run());
    assert_ne!(again.run(), resumed);
}

/// The Run's id is the caller's on this path too, and one episode acquiring
/// two starts is not something a primary key should be left to report.
#[test]
fn opening_a_managed_session_refuses_a_run_id_the_log_holds() {
    let mut store = TestStore::new("managed-run-repeat");
    let taken = opened(&mut store, &command("cmd-1", "/work"), instant(10)).expect("created");

    let refusal = store
        .start_managed_session(
            &command("cmd-2", "/elsewhere"),
            CorralSessionId::mint(),
            taken.run(),
            OccurrenceTime::Authoritative(instant(20)),
            instant(20),
        )
        .expect_err("refused");

    assert!(matches!(
        refusal,
        StateError::Refused(Refusal::RunAlreadyRecorded(named)) if named == taken.run()
    ));
    assert_eq!(store.sessions().expect("readable").len(), 1);
}

/// The consult that runs before anything is spawned refuses a command the
/// commit could never take, so an impossible command does not first cost a
/// process and then a teardown.
#[test]
fn an_oversized_command_is_refused_before_it_is_looked_up() {
    let mut store = TestStore::new("oversize-consult");
    let huge = Command::new(
        CommandId::new("cmd-1").expect("usable"),
        CommandFingerprint::builder(CommandKind::new("session.new").expect("usable"))
            .input("argv.0", "x".repeat(8192))
            .build(),
    );

    let refusal = store.completed_managed_session(&huge).expect_err("refused");

    assert!(matches!(
        refusal,
        StateError::Refused(Refusal::FingerprintTooLarge { .. })
    ));
}

// ---------------------------------------------------------------------------
// Identity conflict: the one durable fact this phase adds (ADR 0004 D8).
// ---------------------------------------------------------------------------

/// A Session with an Attested provider-session binding, the way a managed
/// launch's first `SessionStart` leaves it.
fn attested_provider_session(store: &mut Store, external: &str) -> (CorralSessionId, BindingId) {
    let (session, _) = managed_session(store, &format!("runtime-for-{external}"));
    let node = store.node();
    let BindingResolution::Created(binding) = store
        .bind(
            session,
            key(node, BindingKind::ProviderSession, external),
            Provenance::Discovered,
            evidence(EvidenceSource::ProviderHook, Assurance::Attested),
            instant(20),
        )
        .expect("bound")
    else {
        panic!("a new provider identity is a new binding");
    };
    (session, binding.id())
}

#[test]
fn a_conflicting_identity_report_contests_the_binding_it_contradicts() {
    let mut store = TestStore::new("contest");
    let (session, binding) = attested_provider_session(&mut store, "sess-x");

    let outcome = store
        .contest_binding(
            binding,
            ExternalId::new("sess-y").expect("usable"),
            evidence(EvidenceSource::ProviderHook, Assurance::Attested),
        )
        .expect("contested");

    assert!(matches!(outcome, Contested::Recorded(_)));
    assert_eq!(
        outcome.binding().identity_status(),
        IdentityStatus::Contested
    );
    assert_eq!(
        kinds(&store.events_of(session).expect("readable")),
        vec![
            "session-created",
            "binding-added",
            "binding-added",
            "binding-contested",
        ],
    );
}

/// Contested is monotonic. A runtime that keeps naming a disputed identity
/// must not grow the log by one transition event per hook, and later reports
/// of either id change nothing.
#[test]
fn contesting_twice_records_one_fact() {
    let mut store = TestStore::new("contest-monotonic");
    let (session, binding) = attested_provider_session(&mut store, "sess-x");
    let contest = |store: &mut Store, other: &str| {
        store
            .contest_binding(
                binding,
                ExternalId::new(other).expect("usable"),
                evidence(EvidenceSource::ProviderHook, Assurance::Attested),
            )
            .expect("answered")
    };

    assert!(matches!(
        contest(&mut store, "sess-y"),
        Contested::Recorded(_)
    ));
    // The conflicting id again, and a third: neither is a transition, and
    // neither clears anything.
    for reported in ["sess-y", "sess-z"] {
        let repeat = contest(&mut store, reported);
        assert!(matches!(repeat, Contested::Already(_)), "{reported}");
        assert_eq!(
            repeat.binding().identity_status(),
            IdentityStatus::Contested
        );
    }
    // The original id contradicts nothing, contested or not, so it is refused
    // rather than absorbed: the store asks whether there is a conflict before
    // it asks whether one is already recorded.
    let agrees = store
        .contest_binding(
            binding,
            ExternalId::new("sess-x").expect("usable"),
            evidence(EvidenceSource::ProviderHook, Assurance::Attested),
        )
        .expect_err("refused");
    assert!(matches!(
        agrees,
        StateError::Refused(Refusal::IdentityDoesNotConflict { .. })
    ));

    let contests = kinds(&store.events_of(session).expect("readable"))
        .into_iter()
        .filter(|kind| *kind == "binding-contested")
        .count();
    assert_eq!(contests, 1);
}

/// The contest records that an identifier was reported. It creates no binding
/// to it, and the Session keeps exactly the identity it had.
#[test]
fn a_contest_creates_no_binding_to_the_conflicting_identity() {
    let mut store = TestStore::new("contest-no-binding");
    let (session, binding) = attested_provider_session(&mut store, "sess-x");
    let node = store.node();

    store
        .contest_binding(
            binding,
            ExternalId::new("sess-y").expect("usable"),
            evidence(EvidenceSource::ProviderHook, Assurance::Attested),
        )
        .expect("contested");

    let held = store.bindings_of(session).expect("readable");
    assert_eq!(held.len(), 2, "the runtime binding and the one identity");
    assert!(
        held.iter()
            .all(|binding| binding.key() != &key(node, BindingKind::ProviderSession, "sess-y")),
        "the reported identifier is not bound to anything",
    );
}

/// Assurance stays orthogonal: Attested-and-contested is not Heuristic, and
/// the evidence the binding rests on is not overwritten by the contest.
#[test]
fn a_contest_leaves_assurance_and_evidence_alone() {
    let mut store = TestStore::new("contest-orthogonal");
    let (_, binding) = attested_provider_session(&mut store, "sess-x");
    let before = store.binding(binding).expect("readable").expect("held");

    store
        .contest_binding(
            binding,
            ExternalId::new("sess-y").expect("usable"),
            Evidence::new(
                EvidenceSource::ProviderHook,
                Assurance::Attested,
                instant(900),
            ),
        )
        .expect("contested");

    let after = store.binding(binding).expect("readable").expect("held");
    assert_eq!(after.assurance(), Assurance::Attested);
    assert_eq!(after.evidence(), before.evidence());
    assert_eq!(after.identity_status(), IdentityStatus::Contested);
}

/// Contested survives a restart by construction. A contest that evaporated
/// would let the next continuation act on an identity Corral already knows is
/// disputed — which is the whole reason it is durable rather than a flag.
#[test]
fn a_contest_survives_the_process_that_recorded_it() {
    let mut store = TestStore::new("contest-durable");
    let (_, binding) = attested_provider_session(&mut store, "sess-x");
    store
        .contest_binding(
            binding,
            ExternalId::new("sess-y").expect("usable"),
            evidence(EvidenceSource::ProviderHook, Assurance::Attested),
        )
        .expect("contested");

    store.reopen();

    let read = store.binding(binding).expect("readable").expect("held");
    assert_eq!(read.identity_status(), IdentityStatus::Contested);
    assert_eq!(
        read.native_resume_eligibility(),
        corral_core::NativeResumeEligibility::IdentityContested,
    );
}

/// Projections are a summary of the log and never more. Clearing them and
/// replaying has to reproduce a contest exactly.
#[test]
fn a_rebuild_reproduces_a_contest() {
    let mut store = TestStore::new("contest-rebuild");
    let (_, binding) = attested_provider_session(&mut store, "sess-x");
    store
        .contest_binding(
            binding,
            ExternalId::new("sess-y").expect("usable"),
            evidence(EvidenceSource::ProviderHook, Assurance::Attested),
        )
        .expect("contested");
    let before = store.binding(binding).expect("readable");

    store.rebuild_projections().expect("rebuilt");

    assert_eq!(store.binding(binding).expect("readable"), before);
}

/// A durable, unclearable fact may not rest on a guess.
#[test]
fn heuristic_evidence_does_not_contest_a_binding() {
    let mut store = TestStore::new("contest-weak");
    let (_, binding) = attested_provider_session(&mut store, "sess-x");

    let refusal = store
        .contest_binding(
            binding,
            ExternalId::new("sess-y").expect("usable"),
            evidence(EvidenceSource::Correlation, Assurance::Heuristic),
        )
        .expect_err("refused");

    assert!(matches!(
        refusal,
        StateError::Refused(Refusal::UnsupportedContest { .. })
    ));
}

/// The claim that can become ambiguous is which provider conversation a
/// Session names. A managed runtime's identity is Corral-minted, and nothing a
/// provider says can contradict it (ADR 0008 D2).
#[test]
fn a_managed_runtime_binding_cannot_be_contested() {
    let mut store = TestStore::new("contest-runtime");
    let (_, runtime) = managed_session(&mut store, "run-a");

    let refusal = store
        .contest_binding(
            runtime,
            ExternalId::new("sess-y").expect("usable"),
            evidence(EvidenceSource::ProviderHook, Assurance::Attested),
        )
        .expect_err("refused");

    assert!(matches!(
        refusal,
        StateError::Refused(Refusal::NotAProviderSessionBinding(named)) if named == runtime
    ));
}

#[test]
fn contesting_a_binding_the_log_does_not_hold_is_refused() {
    let mut store = TestStore::new("contest-unknown");

    let refusal = store
        .contest_binding(
            BindingId::mint(),
            ExternalId::new("sess-y").expect("usable"),
            evidence(EvidenceSource::ProviderHook, Assurance::Attested),
        )
        .expect_err("refused");

    assert!(matches!(
        refusal,
        StateError::Refused(Refusal::UnknownBinding(_))
    ));
}

// ---------------------------------------------------------------------------
// Continuation: the same Session, a new Run, the same runtime binding.
// ---------------------------------------------------------------------------

fn resume(
    store: &mut Store,
    command: &Command,
    session: CorralSessionId,
    at: SystemTime,
) -> Result<StartedManagedSession, StateError> {
    store.resume_managed_session(
        command,
        session,
        RunId::mint(),
        OccurrenceTime::Authoritative(at),
        at,
    )
}

/// One Session has one managed-runtime binding and many Runs under it, so a
/// continuation reuses the binding rather than minting a second (ADR 0008 D2).
#[test]
fn a_continuation_reuses_the_managed_runtime_binding() {
    let mut store = TestStore::new("resume-binding");
    let first = opened(&mut store, &command("cmd-1", "/work"), instant(10)).expect("created");
    store
        .record_run_ended(
            first.run(),
            RunEnd::Exited(ExitCause::Completed),
            OccurrenceTime::Authoritative(instant(20)),
        )
        .expect("ended");

    let again = resume(
        &mut store,
        &command("cmd-2", "/work"),
        first.session(),
        instant(30),
    )
    .expect("resumed");

    assert_eq!(again.session(), first.session());
    assert_ne!(again.run(), first.run());
    assert_eq!(
        store.bindings_of(first.session()).expect("readable").len(),
        1
    );
    let runs = store.runs_of(first.session()).expect("readable");
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].runtime_binding(), runs[1].runtime_binding());
}

/// A retry replays its own receipt, and a continuation's receipt names the Run
/// it made — not the Session's first one.
#[test]
fn a_repeated_continuation_replays_the_run_it_made() {
    let mut store = TestStore::new("resume-replay");
    let first = opened(&mut store, &command("cmd-1", "/work"), instant(10)).expect("created");
    store
        .record_run_ended(
            first.run(),
            RunEnd::Exited(ExitCause::Completed),
            OccurrenceTime::Authoritative(instant(20)),
        )
        .expect("ended");
    let again = resume(
        &mut store,
        &command("cmd-2", "/work"),
        first.session(),
        instant(30),
    )
    .expect("resumed");

    let replayed = resume(
        &mut store,
        &command("cmd-2", "/work"),
        first.session(),
        instant(40),
    )
    .expect("replayed");

    assert!(matches!(
        replayed.acceptance(),
        CommandAcceptance::Replayed(_)
    ));
    assert_eq!(replayed.session(), again.session());
    assert_eq!(replayed.run(), again.run(), "the run this command made");
    assert_eq!(store.runs_of(first.session()).expect("readable").len(), 2);
}

/// The store's own invariant, enforced where it lives: one runtime is one
/// episode at a time.
#[test]
fn a_continuation_of_a_still_running_session_is_refused() {
    let mut store = TestStore::new("resume-live");
    let first = opened(&mut store, &command("cmd-1", "/work"), instant(10)).expect("created");

    let refusal = resume(
        &mut store,
        &command("cmd-2", "/work"),
        first.session(),
        instant(20),
    )
    .expect_err("refused");

    assert!(matches!(
        refusal,
        StateError::Refused(Refusal::RunAlreadyLive { run, .. }) if run == first.run()
    ));
}

#[test]
fn a_continuation_of_a_session_the_log_does_not_hold_is_refused() {
    let mut store = TestStore::new("resume-unknown");

    let refusal = resume(
        &mut store,
        &command("cmd-1", "/work"),
        CorralSessionId::mint(),
        instant(10),
    )
    .expect_err("refused");

    assert!(matches!(
        refusal,
        StateError::Refused(Refusal::UnknownSession(_))
    ));
}

/// A Session with no control-capable runtime binding has nothing to file
/// another Run's association under, and minting a second here would break the
/// at-most-one rule from the other side.
#[test]
fn a_continuation_needs_a_runtime_binding_to_belong_to() {
    let mut store = TestStore::new("resume-no-binding");
    let node = store.node();
    let SessionResolution::Created { session, .. } = store
        .resolve_or_create_session(
            key(node, BindingKind::ProviderSession, "sess-only"),
            Provenance::Discovered,
            evidence(EvidenceSource::ProviderHook, Assurance::Attested),
            instant(10),
        )
        .expect("resolved")
    else {
        panic!("a new external identity is a new Session");
    };

    let refusal = resume(
        &mut store,
        &command("cmd-1", "/work"),
        session.id(),
        instant(20),
    )
    .expect_err("refused");

    assert!(matches!(
        refusal,
        StateError::Refused(Refusal::NoManagedRuntimeBinding(named)) if named == session.id()
    ));
}

/// A contest needs something to contest. Recorded against the identity a
/// binding already holds it would take a Session's continuation away for good
/// over an agreement — and contested is monotonic, so there is no way back.
#[test]
fn contesting_a_binding_with_its_own_identity_is_refused() {
    let mut store = TestStore::new("contest-agrees");
    let (session, binding) = attested_provider_session(&mut store, "sess-x");

    let refusal = store
        .contest_binding(
            binding,
            ExternalId::new("sess-x").expect("usable"),
            evidence(EvidenceSource::ProviderHook, Assurance::Attested),
        )
        .expect_err("refused");

    assert!(matches!(
        refusal,
        StateError::Refused(Refusal::IdentityDoesNotConflict { binding: named })
            if named == binding
    ));
    assert!(!kinds(&store.events_of(session).expect("readable")).contains(&"binding-contested"));
    assert_eq!(
        store
            .binding(binding)
            .expect("readable")
            .expect("held")
            .identity_status(),
        IdentityStatus::Confirmed,
    );
}

/// Deliveries arrive on their own connections and are stamped at their own
/// arrival, so an order that disagrees with the stamps is ordinary scheduling.
/// A stale re-observation tells the log nothing, and letting it land would
/// move the binding's freshness backwards — which is what later phases judge
/// what a fact may still claim by.
#[test]
fn a_stale_re_observation_does_not_move_a_binding_backwards() {
    let mut store = TestStore::new("confirm-stale");
    let (session, binding) = attested_provider_session(&mut store, "sess-x");
    let fresh = Evidence::new(
        EvidenceSource::ProviderHook,
        Assurance::Attested,
        instant(500),
    );
    store.confirm_binding(binding, fresh).expect("confirmed");

    let confirmed = store
        .confirm_binding(
            binding,
            Evidence::new(
                EvidenceSource::ProviderHook,
                Assurance::Attested,
                instant(300),
            ),
        )
        .expect("answered");

    assert_eq!(confirmed.evidence().observed_at(), instant(500));
    assert_eq!(
        kinds(&store.events_of(session).expect("readable"))
            .into_iter()
            .filter(|kind| *kind == "binding-confirmed")
            .count(),
        1,
        "a stale observation was written",
    );
}

/// A promotion is not a re-observation: evidence that changes what the binding
/// may do lands however it is stamped.
#[test]
fn a_stronger_assurance_lands_even_when_it_is_older() {
    let mut store = TestStore::new("confirm-promote");
    let node = store.node();
    let (session, _) = managed_session(&mut store, "run-a");
    let BindingResolution::Created(weak) = store
        .bind(
            session,
            key(node, BindingKind::History, "hist-1"),
            Provenance::Discovered,
            Evidence::new(
                EvidenceSource::Correlation,
                Assurance::Heuristic,
                instant(500),
            ),
            instant(20),
        )
        .expect("bound")
    else {
        panic!("a new identity is a new binding");
    };

    let promoted = store
        .confirm_binding(
            weak.id(),
            Evidence::new(
                EvidenceSource::ProviderHook,
                Assurance::Attested,
                instant(300),
            ),
        )
        .expect("confirmed");

    assert_eq!(promoted.assurance(), Assurance::Attested);
}

/// The promotion exception is a direction, not a licence for any differing
/// assurance to overwrite. Two kinds of evidence that both support control are
/// of equal standing, so the older one is still stale — and letting it land
/// would rewind the freshness of a binding while changing what it claims to
/// rest on.
#[test]
fn an_older_confirmation_of_equal_standing_is_still_stale() {
    let mut store = TestStore::new("confirm-sideways");
    let (_, runtime) = managed_session(&mut store, "run-sideways");

    let answered = store
        .confirm_binding(
            runtime,
            Evidence::new(
                EvidenceSource::ProviderHook,
                Assurance::Attested,
                instant(50),
            ),
        )
        .expect("answered");

    assert_eq!(answered.assurance(), Assurance::Deterministic);
    assert_eq!(answered.evidence().observed_at(), instant(100));
}

// Integration intent and repair authority: Corral-owned facts the event log is
// deliberately not the carrier of (ADR 0013 D6, grill Q4′).

fn claude() -> ProviderId {
    ProviderId::new("claude").expect("usable")
}

fn missing_entry() -> RepairFingerprint {
    RepairFingerprint::new(
        claude(),
        ConfigTarget::ClaudeUserSettings,
        RepairableDrift::Missing,
    )
}

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

fn three_a_day() -> RepairBudget {
    RepairBudget::new(3, DAY)
}

/// Absence is not a decision. A caller that read it as `Disabled` would let a
/// fresh install claim the user opted out.
#[test]
fn a_provider_the_user_never_decided_about_has_no_recorded_intent() {
    let mut store = TestStore::new("intent-absent");

    assert_eq!(store.integration_intent(&claude()).expect("read"), None);
}

#[test]
fn an_integration_decision_survives_the_process_that_made_it() {
    let mut store = TestStore::new("intent-durable");
    store
        .set_integration_intent(&claude(), IntegrationIntent::Disabled, instant(400))
        .expect("record the decision");

    store.reopen();

    let recorded = store
        .integration_intent(&claude())
        .expect("read")
        .expect("a decision");
    assert_eq!(recorded.intent(), IntegrationIntent::Disabled);
    assert_eq!(recorded.changed_at(), instant(400));
}

#[test]
fn deciding_again_replaces_the_decision_rather_than_accumulating_one() {
    let mut store = TestStore::new("intent-replace");
    store
        .set_integration_intent(&claude(), IntegrationIntent::Enabled, instant(100))
        .expect("enable");
    store
        .set_integration_intent(&claude(), IntegrationIntent::Disabled, instant(200))
        .expect("disable");

    let recorded = store
        .integration_intent(&claude())
        .expect("read")
        .expect("a decision");
    assert_eq!(recorded.intent(), IntegrationIntent::Disabled);
    assert_eq!(recorded.changed_at(), instant(200));
}

/// Rebuilding projections must not touch a user decision: the log never
/// carried it, so a replay that cleared it would forget rather than recompute.
#[test]
fn rebuilding_projections_leaves_integration_intent_alone() {
    let mut store = TestStore::new("intent-rebuild");
    store
        .set_integration_intent(&claude(), IntegrationIntent::Disabled, instant(300))
        .expect("record");

    store.rebuild_projections().expect("rebuild");

    let recorded = store
        .integration_intent(&claude())
        .expect("read")
        .expect("a decision");
    assert_eq!(recorded.intent(), IntegrationIntent::Disabled);
}

#[test]
fn a_budget_admits_its_repairs_and_then_withdraws_authority() {
    let mut store = TestStore::new("repair-budget");
    let fingerprint = missing_entry();

    for repair in 0..3 {
        let authority = store
            .authorize_repair(&fingerprint, instant(1_000 + repair), three_a_day())
            .expect("authorize");
        assert_eq!(
            authority,
            RepairAuthority::Available {
                remaining: 3 - u32::try_from(repair).expect("small"),
            }
        );
        store
            .record_repair(&fingerprint, instant(1_000 + repair))
            .expect("record");
    }

    let fourth = store
        .authorize_repair(&fingerprint, instant(1_003), three_a_day())
        .expect("authorize");
    assert_eq!(
        fourth,
        RepairAuthority::Withdrawn {
            since: instant(1_003)
        }
    );
    assert!(!fourth.permits_repair());
}

/// The whole point of the sticky breaker: a dotfiles authority that repaints
/// the file once a day must not get a fresh repair every day.
#[test]
fn a_withdrawn_authority_does_not_return_when_the_window_slides_past_it() {
    let mut store = TestStore::new("repair-sticky-window");
    let fingerprint = missing_entry();
    for repair in 0..3 {
        store
            .authorize_repair(&fingerprint, instant(1_000 + repair), three_a_day())
            .expect("authorize");
        store
            .record_repair(&fingerprint, instant(1_000 + repair))
            .expect("record");
    }
    store
        .authorize_repair(&fingerprint, instant(1_003), three_a_day())
        .expect("open the breaker");

    let a_week_later = instant(1_003 + 7 * DAY.as_secs());
    let authority = store
        .authorize_repair(&fingerprint, a_week_later, three_a_day())
        .expect("authorize");

    assert_eq!(
        authority,
        RepairAuthority::Withdrawn {
            since: instant(1_003)
        }
    );
}

#[test]
fn a_daemon_restart_does_not_re_arm_a_withdrawn_authority() {
    let mut store = TestStore::new("repair-sticky-restart");
    let fingerprint = missing_entry();
    for repair in 0..3 {
        store
            .authorize_repair(&fingerprint, instant(1_000 + repair), three_a_day())
            .expect("authorize");
        store
            .record_repair(&fingerprint, instant(1_000 + repair))
            .expect("record");
    }
    store
        .authorize_repair(&fingerprint, instant(1_003), three_a_day())
        .expect("open the breaker");

    store.reopen();

    let authority = store
        .authorize_repair(&fingerprint, instant(1_004), three_a_day())
        .expect("authorize");
    assert_eq!(
        authority,
        RepairAuthority::Withdrawn {
            since: instant(1_003)
        }
    );
}

#[test]
fn an_explicit_reconciliation_is_what_re_arms_repair() {
    let mut store = TestStore::new("repair-restore");
    let fingerprint = missing_entry();
    for repair in 0..3 {
        store
            .authorize_repair(&fingerprint, instant(1_000 + repair), three_a_day())
            .expect("authorize");
        store
            .record_repair(&fingerprint, instant(1_000 + repair))
            .expect("record");
    }
    store
        .authorize_repair(&fingerprint, instant(1_003), three_a_day())
        .expect("open the breaker");

    store
        .restore_repair_authority(&fingerprint)
        .expect("reconcile");

    let authority = store
        .authorize_repair(&fingerprint, instant(1_004), three_a_day())
        .expect("authorize");
    assert_eq!(authority, RepairAuthority::Available { remaining: 3 });
}

/// Repairs that fell out of the window are forgotten, so a provider that drops
/// Corral's entry once a month is repaired every time.
#[test]
fn repairs_older_than_the_window_do_not_count_against_the_budget() {
    let mut store = TestStore::new("repair-window");
    let fingerprint = missing_entry();
    for repair in 0..3 {
        store
            .authorize_repair(&fingerprint, instant(1_000 + repair), three_a_day())
            .expect("authorize");
        store
            .record_repair(&fingerprint, instant(1_000 + repair))
            .expect("record");
    }

    let next_month = instant(1_000 + 30 * DAY.as_secs());
    let authority = store
        .authorize_repair(&fingerprint, next_month, three_a_day())
        .expect("authorize");

    assert_eq!(authority, RepairAuthority::Available { remaining: 3 });
}

/// Two drift classes in one file are two recurrences. A provider rewrite
/// eating the entry must not spend the budget an upgrade's stale
/// representation needs.
#[test]
fn one_drift_class_cannot_exhaust_anothers_budget() {
    let mut store = TestStore::new("repair-fingerprint");
    let missing = missing_entry();
    let stale = RepairFingerprint::new(
        claude(),
        ConfigTarget::ClaudeUserSettings,
        RepairableDrift::OldRepresentation,
    );
    for repair in 0..3 {
        store
            .authorize_repair(&missing, instant(1_000 + repair), three_a_day())
            .expect("authorize");
        store
            .record_repair(&missing, instant(1_000 + repair))
            .expect("record");
    }
    store
        .authorize_repair(&missing, instant(1_003), three_a_day())
        .expect("open the breaker");

    let authority = store
        .authorize_repair(&stale, instant(1_004), three_a_day())
        .expect("authorize");

    assert_eq!(authority, RepairAuthority::Available { remaining: 3 });
}

fn continuation() -> Command {
    Command::new(
        CommandId::new(CorralSessionId::mint().to_string()).expect("usable"),
        CommandFingerprint::builder(CommandKind::new("session.resume").expect("usable"))
            .input("session", "discovered")
            .build(),
    )
}

/// A discovered runtime is somebody else's process. Corral holds no handle on
/// it, so a continuation must never hang a managed Run under it — and, once
/// the external Run has ended, that is exactly what picking it by assurance
/// alone would do (ADR 0014 D6).
#[test]
fn a_continuation_never_lands_on_a_runtime_corral_only_discovered() {
    let mut store = TestStore::new("discovered-runtime");
    let node = store.node();
    let discovered = store
        .resolve_or_create_session(
            key(node, BindingKind::ProviderSession, "provider-session-1"),
            Provenance::Discovered,
            Evidence::new(
                EvidenceSource::ProviderHook,
                Assurance::Attested,
                instant(100),
            ),
            instant(100),
        )
        .expect("resolve");
    let session = match discovered {
        SessionResolution::Created { session, .. } => session.id(),
        SessionResolution::Existing { session, .. } => session.id(),
    };
    store
        .bind(
            session,
            key(node, BindingKind::Runtime, "pid-4321-500000000"),
            Provenance::Discovered,
            Evidence::new(
                EvidenceSource::NodeRuntimeObservation,
                Assurance::Attested,
                instant(100),
            ),
            instant(100),
        )
        .expect("bind the discovered runtime");

    let continued = store.resume_managed_session(
        &continuation(),
        session,
        RunId::mint(),
        OccurrenceTime::Authoritative(instant(200)),
        instant(200),
    );

    assert!(
        matches!(
            continued,
            Err(StateError::Refused(Refusal::NoManagedRuntimeBinding(named))) if named == session
        ),
        "a discovered runtime was offered as a continuation target: {continued:?}",
    );
}

/// The other half of the same defect: admitting the managed runtime binding a
/// continuation actually needs must not be refused because a discovered one is
/// already there.
#[test]
fn a_discovered_runtime_does_not_block_the_managed_binding_that_belongs_there() {
    let mut store = TestStore::new("discovered-does-not-block");
    let node = store.node();
    let discovered = store
        .resolve_or_create_session(
            key(node, BindingKind::ProviderSession, "provider-session-2"),
            Provenance::Discovered,
            Evidence::new(
                EvidenceSource::ProviderHook,
                Assurance::Attested,
                instant(100),
            ),
            instant(100),
        )
        .expect("resolve");
    let session = match discovered {
        SessionResolution::Created { session, .. } => session.id(),
        SessionResolution::Existing { session, .. } => session.id(),
    };
    store
        .bind(
            session,
            key(node, BindingKind::Runtime, "pid-4321-500000000"),
            Provenance::Discovered,
            Evidence::new(
                EvidenceSource::NodeRuntimeObservation,
                Assurance::Attested,
                instant(100),
            ),
            instant(100),
        )
        .expect("bind the discovered runtime");

    let managed = store.bind(
        session,
        managed_key(node, "managed-runtime-1"),
        Provenance::CorralCreated,
        Evidence::new(
            EvidenceSource::CorralConstructed,
            Assurance::Deterministic,
            instant(200),
        ),
        instant(200),
    );

    assert!(
        managed.is_ok(),
        "a discovered runtime blocked the managed one: {managed:?}",
    );
}

/// The at-most-one rule counts control-capable runtime bindings. A discovered
/// one is not one, so a Session may hold Corral's own runtime and a process
/// discovery found for it at the same time — which is the ordinary outcome
/// when the global integration entry fires for a managed session.
#[test]
fn a_discovered_runtime_is_admitted_beside_the_managed_one() {
    let mut store = TestStore::new("discovered-beside-managed");
    let node = store.node();
    let (session, _) = managed_session(&mut store, "run-a");

    let admitted = store.bind(
        session,
        key(node, BindingKind::Runtime, "pid-4321-500000000"),
        Provenance::Discovered,
        evidence(EvidenceSource::NodeRuntimeObservation, Assurance::Attested),
        instant(12),
    );

    assert!(admitted.is_ok(), "{admitted:?}");
}

/// A history enumeration resolves an identity against every binding kind
/// before it makes a row (ADR 0016 D2): a Session bound to a provider
/// session id is found by that id whether the lookup names history or not,
/// and an unknown id is nobody's.
#[test]
fn a_session_is_found_by_its_external_id_whatever_the_binding_kind() {
    let mut store = TestStore::new("by-external-id");
    let node = store.node();
    let SessionResolution::Created { session, .. } = store
        .resolve_or_create_session(
            key(node, BindingKind::ProviderSession, "session-abc"),
            Provenance::Discovered,
            owned_runtime(),
            instant(10),
        )
        .expect("resolved")
    else {
        panic!("a new external identity is a new Session");
    };

    let provider = ProviderId::new("claude-code").expect("usable");
    let found = store
        .session_by_external_id(&provider, &ExternalId::new("session-abc").expect("usable"))
        .expect("readable");
    assert_eq!(found, Some(session.id()));

    let unknown = store
        .session_by_external_id(&provider, &ExternalId::new("session-xyz").expect("usable"))
        .expect("readable");
    assert_eq!(unknown, None);
}

/// Continuing a session Corral knows only from a provider's own store puts it
/// in the durable log for the first time: the Session, the `HistoryBinding`
/// that says which provider session it is, the managed runtime Corral is
/// about to own, and the Run — one transaction, or nothing (ADR 0016 D2).
#[test]
fn continuing_a_history_row_records_the_session_its_history_binding_and_its_run() {
    let mut store = TestStore::new("continue-history");
    let at = instant(500);
    // The pass that read the store ran before the write that records it.
    let observed = instant(300);
    let session = CorralSessionId::mint();
    let run = RunId::mint();
    let history = BindingKey::history(
        store.node(),
        ProviderId::new("claude-code").expect("usable"),
        ExternalId::new("session-abc").expect("usable"),
    );

    let started = store
        .continue_history_session(
            &command("continue-1", "/w"),
            session,
            run,
            HistoryObservation {
                key: history.clone(),
                observed_at: observed,
            },
            OccurrenceTime::Authoritative(at),
            at,
        )
        .expect("recorded");

    assert_eq!(started.session, session);
    assert_eq!(started.run, run);
    assert_eq!(
        kinds(&store.events_of(session).expect("read")),
        vec![
            "session-created",
            "binding-added",
            "binding-added",
            "run-started",
            "command-accepted"
        ]
    );

    // The history binding claims the identity and nothing about the present:
    // Attested for what the store holds, from the store's own record, and
    // never control-capable (ADR 0016 D3).
    let bindings = store.bindings_of(session).expect("read");
    let history = bindings
        .iter()
        .find(|binding| binding.key().kind() == BindingKind::History)
        .expect("a history binding");
    assert_eq!(history.key().kind(), BindingKind::History);
    assert_eq!(history.key().external_id().as_str(), "session-abc");
    assert_eq!(history.assurance(), Assurance::Attested);
    assert_eq!(history.evidence().source(), EvidenceSource::HistoryRecord);
    // Dated on the pass that read the store, not on this write: freshness
    // asks how old the observation is (ADR 0015 D5).
    assert_eq!(history.evidence().observed_at(), observed);
    assert_eq!(history.provenance(), Provenance::Discovered);
    assert!(!history.is_control_capable_runtime_binding());

    // And the Run belongs to the managed runtime, which is what Corral drives.
    let runs = store.runs_of(session).expect("read");
    let managed = bindings
        .iter()
        .find(|binding| binding.is_control_capable_runtime_binding())
        .expect("a managed runtime binding");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].runtime_binding(), managed.id());

    // The store found it by the identity the provider's store named.
    assert_eq!(
        store
            .session_by_external_id(
                &ProviderId::new("claude-code").expect("usable"),
                &ExternalId::new("session-abc").expect("usable")
            )
            .expect("read"),
        Some(session)
    );
}

/// The same continuation sent twice is one continuation: the receipt answers
/// the retry, and no second Session appears for the same provider session.
#[test]
fn a_retried_history_continuation_replays_its_receipt() {
    let mut store = TestStore::new("continue-history-retry");
    let at = instant(500);
    let observed = instant(300);
    let command = command("continue-1", "/w");
    let history = BindingKey::history(
        store.node(),
        ProviderId::new("claude-code").expect("usable"),
        ExternalId::new("session-abc").expect("usable"),
    );
    let first = store
        .continue_history_session(
            &command,
            CorralSessionId::mint(),
            RunId::mint(),
            HistoryObservation {
                key: history.clone(),
                observed_at: observed,
            },
            OccurrenceTime::Authoritative(at),
            at,
        )
        .expect("recorded");

    let again = store
        .continue_history_session(
            &command,
            CorralSessionId::mint(),
            RunId::mint(),
            HistoryObservation {
                key: history.clone(),
                observed_at: observed,
            },
            OccurrenceTime::Authoritative(at),
            at,
        )
        .expect("replayed");

    assert_eq!(again.session, first.session);
    assert_eq!(again.run, first.run);
    assert_eq!(
        store
            .events_of(first.session)
            .expect("read")
            .iter()
            .filter(|recorded| recorded.event().kind() == "session-created")
            .count(),
        1,
        "a retry created a second Session"
    );
}
