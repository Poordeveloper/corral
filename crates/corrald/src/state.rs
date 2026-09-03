use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use corral_core::{
    Binding, BindingKey, BindingKind, Command, CorralSessionId, Evidence, EvidenceSource,
    ExternalId, IntegrationIntent, NodeId, OccurrenceTime, Provenance, ProviderId, RepairAuthority,
    RepairFingerprint, Run, RunId,
};
use corral_state::{
    BindingResolution, Contested, FatalState, RecordedIntent, RecordedRun, Refusal,
    SessionResolution, StartedManagedSession, StateError, Store,
};

use crate::hook_evidence::{Deliveries, Ingest};
use crate::in_flight::InFlightCommands;
use crate::policy;
use crate::provider::{ReportedSessions, SharedLaunchTokens};
use crate::runtime::{
    AttachTokens, Integrity, ManagedSessions, OwnedChildren, RunObservations, observe_runs,
};

/// How long a departing daemon waits for its last observed facts to land.
///
/// Derived from the recorder's own budget rather than chosen beside it. The
/// recorder legitimately waits out a store another writer is holding, and a
/// shutdown that gave up first would declare a hole in the accounting — and
/// exit non-zero — while the write was still going to succeed.
const SETTLE_GRACE: Duration = Duration::from_millis(
    crate::run_lifecycle::LONGEST_OCCURRENCE.as_millis() as u64 + STORE_WAIT_OVERSHOOT_MILLIS,
);

/// The recorder's budget bounds when it stops *starting* attempts; the attempt
/// under way when it runs out still has the store's own wait to spend.
const STORE_WAIT_OVERSHOOT_MILLIS: u64 = 5_000;

/// What the registry said when asked whether it can still vouch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vouched {
    Yes,
    /// Held by another writer for longer than the store waits. Nothing is
    /// wrong with it; the same request may be sent again.
    NotNow,
}

/// The daemon's one handle on durable state.
///
/// One `Store` behind one lock: the registry is the account's shared truth,
/// and two handles to the same file would be two writers to one log.
///
/// The store is synchronous and can wait on another process holding the
/// database, and `corrald` runs one runtime thread — so every call goes to the
/// blocking pool. On the reactor thread a contended registry would stall every
/// other connection, the idle watchdog, and the signal handler along with it.
pub struct DaemonState {
    /// Behind an `Arc` because the store has a second owner: the thread that
    /// records what the runtime observed. It is the same store — one log, one
    /// writer — reached without going through this type, so a session's
    /// teardown never waits on anything a connection is doing.
    store: Arc<Mutex<Store>>,

    /// Set when this daemon concluded it can no longer vouch for durable truth
    /// by a route the store itself never saw: a call that did not complete.
    /// The store latches its own conclusions; the exit status reads both.
    cannot_vouch: AtomicBool,

    /// Whether this daemon can receive what a managed agent reports.
    ///
    /// Set once, at startup, and only to `false`: the endpoint is bound before
    /// anything can ask for a session, so a client never sees the answer
    /// change under it. A daemon that could not bind still serves everything
    /// else — what it may not do is start a session whose whole point is to
    /// report through an endpoint that is not there.
    hook_endpoint_bound: AtomicBool,

    /// Where the runtime reports what it saw, and the accounting that says
    /// whether all of it was recorded.
    observations: RunObservations,

    /// The launches whose hook events this daemon can still place.
    ///
    /// Beside the runtime rather than inside it, because it has a second
    /// owner: the thread that learns a Run ended drops that Run's token there
    /// (ADR 0004 D5). A token that outlived its Run is only a way to be wrong.
    launch_tokens: SharedLaunchTokens,

    /// This node's identity, read once at startup. Every binding key is scoped
    /// by it, and asking the store for it on each ingest would be a lock taken
    /// for a value that cannot change.
    node: NodeId,

    /// Where per-launch provider configuration Corral owns is written.
    launch_dir: PathBuf,

    /// Corral's own state directory. The integration engine puts a copy of a
    /// user's configuration here before it changes it, which is Corral's
    /// artifact to keep and never something written beside the user's file.
    state_dir: PathBuf,

    /// The provider runtimes the sweep believes are running. Live state: a
    /// restart forgets them and the next pass rediscovers whatever is still
    /// there (ADR 0014 D5).
    seen_runtimes: crate::sweep::SharedSeenRuntimes,

    /// The per-provider turn to mutate a user's configuration, taken by every
    /// operation that records intent and writes the file after it.
    integration_turns: crate::integration::WriteTurns,

    /// Where the hook endpoint puts what it received, and the receiver the
    /// server hands to the one task that interprets it.
    deliveries: Deliveries,
    incoming: Mutex<Option<tokio::sync::mpsc::Receiver<Ingest>>>,

    /// The Sessions a continuation is currently being performed for.
    ///
    /// The in-flight command table dedupes by command id, which is exactly
    /// right for a retry and no protection at all against two *different*
    /// commands continuing one Session: both would read "nothing is running",
    /// both would spawn, and the store would refuse the loser only after a
    /// second provider process was already alive against the same
    /// conversation. That is the outcome grill Q7 forbids, so continuation is
    /// serialized per Session across the whole read-spawn-commit window.
    resuming: Mutex<HashSet<CorralSessionId>>,

    /// The mutating commands this daemon is executing right now.
    commands: InFlightCommands,
    /// The screen-detection manifests this daemon runs with: the built-ins,
    /// and whatever the state directory's `manifests/` overrode at startup
    /// (ADR 0015 D6). Read once; a changed manifest means a restart.
    detection: crate::detection::Loadout,

    /// The account home the providers keep their session stores under, once
    /// the daemon knows it. Absent means no store is enumerated.
    provider_home: Mutex<Option<PathBuf>>,

    /// The attention journal, once the daemon has a diagnostics directory to
    /// put it in. Absent means nothing is journaled — a test daemon, or a
    /// directory that could not be made — and derivation carries on either
    /// way: diagnostics never gate product state.
    journal: Mutex<Option<crate::attention::Journal>>,
    /// The sessions this daemon is running, and the tokens it has issued for
    /// their terminals.
    ///
    /// Live runtime state, deliberately beside the store rather than in it:
    /// a running process is runtime-owned truth and is never persisted as
    /// fact (AGENTS.md §Durable state).
    runtime: Mutex<Runtime>,
}

/// The live runtime a daemon owns for the length of its own life.
#[derive(Default)]
pub struct Runtime {
    pub sessions: ManagedSessions,
    /// Every child this daemon spawned and has not reaped, registered at the
    /// spawn rather than with the session: a sweep can meet the process
    /// before its Run is durable and its handle is here.
    pub owned: OwnedChildren,
    pub attach_tokens: AttachTokens,
    /// What providers have reported about those sessions. Live evidence: a
    /// restart loses it and the rows return to bare runtime truth
    /// (ADR 0004 D7).
    pub reported: ReportedSessions,
    /// Every Session's claims and derived attention state. Live for the same
    /// reason: nothing derived is durable, and a restart reads Unknown until a
    /// session acts (ADR 0015 D8).
    pub attention: crate::attention::Ledger,
    /// What the providers' own stores hold that Corral does not otherwise
    /// know, and the recency they record for what it does. Live: a restart
    /// re-enumerates rather than replays (ADR 0016 D2).
    pub history: crate::history::HistoryRows,
}

impl DaemonState {
    /// Open and validate the registry.
    ///
    /// Called before the daemon binds its endpoint, so a store that cannot be
    /// used is a startup failure rather than something discovered a
    /// millisecond after a client's hello succeeded (ADR 0002, Q14).
    pub fn open(registry: &Path, launch_dir: &Path, state_dir: &Path) -> Result<Self, StateError> {
        let store = Store::open(registry)?;
        let node = store.node();
        let store = Arc::new(Mutex::new(store));
        // Started with the store, not with the server: a runtime that could
        // report an ending before anything was draining the channel would fill
        // it and lose the accounting the daemon exists to keep.
        let (observations, observed) = observe_runs();
        let launch_tokens = SharedLaunchTokens::new();
        let (deliveries, incoming) = crate::hook_evidence::queue();
        crate::run_lifecycle::record_observed_runs(
            Arc::clone(&store),
            observed,
            launch_dir.to_path_buf(),
            deliveries.clone(),
            launch_tokens.clone(),
        );
        Ok(Self {
            store,
            cannot_vouch: AtomicBool::new(false),
            hook_endpoint_bound: AtomicBool::new(true),
            observations,
            launch_tokens,
            node,
            launch_dir: launch_dir.to_path_buf(),
            state_dir: state_dir.to_path_buf(),
            seen_runtimes: crate::sweep::SharedSeenRuntimes::new(),
            integration_turns: crate::integration::WriteTurns::default(),
            deliveries,
            incoming: Mutex::new(Some(incoming)),
            resuming: Mutex::new(HashSet::new()),
            commands: InFlightCommands::new(),
            detection: crate::detection::load_built_in(Some(&state_dir.join("manifests"))),
            provider_home: Mutex::new(None),
            journal: Mutex::new(None),
            runtime: Mutex::new(Runtime::default()),
        })
    }

    /// The manifest the screen thread evaluates for this provider, if any.
    pub fn manifest_for(
        &self,
        provider: crate::provider::KnownProvider,
    ) -> Option<Arc<crate::detection::Manifest>> {
        self.detection
            .manifest(provider.as_str())
            .map(|manifest| Arc::new(manifest.clone()))
    }

    /// Tell this daemon where the providers keep their own files.
    ///
    /// The same home the hook installer works in (`corral_rendezvous::
    /// provider_home`, ADR 0013): a provider's settings and its session store
    /// are two files in one place, and reading them out of two different
    /// notions of "home" is how a test comes to prove a layout no
    /// installation has.
    pub fn attach_provider_home(&self, home: PathBuf) {
        if let Ok(mut slot) = self.provider_home.lock() {
            *slot = Some(home);
        }
    }

    pub fn provider_home(&self) -> Option<PathBuf> {
        self.provider_home.lock().ok().and_then(|slot| slot.clone())
    }

    /// Give this daemon its attention journal.
    pub fn attach_journal(&self, journal: crate::attention::Journal) {
        if let Ok(mut slot) = self.journal.lock() {
            *slot = Some(journal);
        }
    }

    /// Let the journal finish the day it is writing, because this daemon is
    /// stopping on purpose rather than dying. Without this, an orderly
    /// shutdown would leave the same sentinel an abrupt death does, and every
    /// restart would mark a day partial that never lost a thing.
    pub fn close_journal(&self) {
        if let Ok(mut slot) = self.journal.lock()
            && let Some(journal) = slot.as_mut()
        {
            journal.close();
        }
    }

    /// Where the journal lives, when this daemon has one.
    pub fn journal_dir(&self) -> Option<std::path::PathBuf> {
        self.journal
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().map(|journal| journal.dir().to_path_buf()))
    }

    /// Append records to the journal, if there is one. Blocking: the one
    /// caller runs off the reactor.
    pub fn journal_append(
        &self,
        now: std::time::SystemTime,
        records: Vec<crate::attention::Record>,
    ) {
        let Ok(mut slot) = self.journal.lock() else {
            return;
        };
        // No journal attached is not a journal that failed: a daemon without
        // one answers an empty report, which is the truth about what it can
        // report.
        let Some(journal) = slot.as_mut() else {
            return;
        };
        for record in records {
            match journal.append(now, record) {
                Ok(crate::attention::Appended::Written) => {}
                Ok(crate::attention::Appended::BudgetExhausted) => {
                    tracing::warn!(
                        "the attention journal's day budget is exhausted; \
                         the day's records stop here and it is marked incomplete"
                    );
                }
                // Already known and already said. Repeating it every record
                // until the day rolls over would bury the one that mattered,
                // and for a day marked by an I/O failure it would name the
                // wrong cause.
                Ok(crate::attention::Appended::DayAlreadyIncomplete) => {}
                Err(source) => {
                    tracing::warn!(%source, "an attention journal record could not be written");
                    // The record is gone, and nothing on disk would say so.
                    // The marker is the only thing that can, and if it cannot
                    // be written either then no report of this journal can
                    // claim to be complete again (ADR 0015 D8).
                    if let Err(marker) = journal.mark_incomplete(now) {
                        tracing::warn!(
                            %marker,
                            "the attention journal could not be marked incomplete; \
                             reporting is refused until the mark lands"
                        );
                    }
                }
            }
        }
    }

    /// Whether a record was lost with nothing on disk to say so, after one
    /// more attempt to put it there.
    ///
    /// Not a flag this process sets and clears: the condition is a day whose
    /// marker has not landed, and the answer is derived from that each time.
    /// A filesystem that recovers therefore turns into a day the report calls
    /// INCOMPLETE — durably, across restarts — instead of a daemon that
    /// refuses forever or, worse, one that forgets on the way up. A journal
    /// lock nobody can take is the same case: nothing can be written and
    /// nothing can be marked.
    ///
    /// A daemon that dies while the marker is still impossible to write
    /// carries nothing forward itself; the day's sentinel does, because it
    /// was on disk before the write failed (ADR 0015 D8).
    pub fn journal_unreportable(&self) -> bool {
        let Ok(mut slot) = self.journal.lock() else {
            return true;
        };
        slot.as_mut()
            .is_some_and(|journal| journal.settle_marks() > 0)
    }

    /// This node's identity.
    pub fn node(&self) -> NodeId {
        self.node
    }

    /// Mint a token for one managed provider launch.
    pub fn mint_launch_token(
        &self,
        scope: crate::provider::LaunchScope,
    ) -> Result<crate::provider::LaunchToken, crate::provider::NoRandomness> {
        self.launch_tokens.mint(scope)
    }

    /// The launch a token names, if this daemon minted it and its Run is not
    /// over.
    pub fn resolve_launch_token(
        &self,
        token: &crate::provider::LaunchToken,
    ) -> Option<crate::provider::LaunchScope> {
        self.launch_tokens.resolve(token)
    }

    /// Drop a token whose launch never became one.
    pub fn forget_launch_token(&self, token: crate::provider::LaunchToken) {
        self.launch_tokens.forget(token);
    }

    /// Retire the token of a Run that is over.
    ///
    /// Called from the one serial ingestion task, behind every event that Run
    /// already delivered, so "after the Run ended" means after rather than
    /// racing it.
    pub fn retire_launch_tokens_of(&self, run: RunId) {
        self.launch_tokens.forget_run(run);
    }

    /// Where per-launch provider configuration Corral owns is written.
    pub fn launch_dir(&self) -> &Path {
        &self.launch_dir
    }

    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    /// Where the hook endpoint puts what it received.
    pub fn deliveries(&self) -> Deliveries {
        self.deliveries.clone()
    }

    /// The delivery stream, taken once by the task that interprets it.
    ///
    /// Once, because ingestion is serial on purpose: two drainers would let
    /// two events race to establish one Session's first provider identity.
    pub fn take_deliveries(&self) -> Option<tokio::sync::mpsc::Receiver<Ingest>> {
        self.incoming.lock().ok().and_then(|mut held| held.take())
    }

    /// Where a managed runtime reports what it observed about its Run.
    pub fn observations(&self) -> &RunObservations {
        &self.observations
    }

    pub fn commands(&self) -> &InFlightCommands {
        &self.commands
    }

    /// Close every managed-runtime episode a departed daemon left open.
    ///
    /// Synchronous and before the endpoint is bound: reconciliation is part of
    /// deciding what this daemon's durable state says, and a client that
    /// connected first could be told about a Run that was about to be closed
    /// behind it (grill Q5).
    pub fn reconcile_managed_runs(&self) -> Result<Vec<RunId>, StateError> {
        self.lock().end_unowned_managed_runs()
    }

    /// Whether the log records this Run as having exited — `None` when it has
    /// never heard of the Run at all.
    ///
    /// Synchronous, because its one caller is the startup sweep, which runs
    /// before there is a reactor to keep free. Three answers rather than two:
    /// "no such Run" and "a Run whose fate is not established" are what decide
    /// whether a Corral-owned artifact may be destroyed, and collapsing them
    /// would destroy the wrong one (grill Q10).
    pub fn exit_established(&self, run: RunId) -> Option<bool> {
        match self.lock().run(run) {
            Ok(Some(run)) => Some(matches!(run.end(), Some(corral_core::RunEnd::Exited(_)))),
            Ok(None) => None,
            // A store that will not answer is not evidence that anything
            // exited. Retained.
            Err(_) => Some(false),
        }
    }

    /// Wait for every observed fact to be recorded, on the way out.
    pub fn settle_observations(&self) -> Integrity {
        self.observations.settle(SETTLE_GRACE)
    }

    /// What this command already did, if it has run before.
    pub async fn completed_managed_session(
        self: &Arc<Self>,
        command: Command,
    ) -> Result<Option<StartedManagedSession>, StateError> {
        self.off_the_reactor(move |store| store.completed_managed_session(&command))
            .await
    }

    /// Record a Session, its managed runtime binding, and its first Run.
    pub async fn start_managed_session(
        self: &Arc<Self>,
        command: Command,
        session: CorralSessionId,
        run: RunId,
        started: OccurrenceTime,
        at: SystemTime,
    ) -> Result<StartedManagedSession, StateError> {
        self.off_the_reactor(move |store| {
            store.start_managed_session(&command, session, run, started, at)
        })
        .await
    }

    /// Claim the right to continue this Session, or find it already claimed.
    ///
    /// Held for the whole of deciding, spawning, and committing — which is the
    /// window the store cannot close on its own, because the Run that would
    /// make it refuse does not exist until the spawn has already happened.
    pub fn claim_continuation(self: &Arc<Self>, session: CorralSessionId) -> Option<Continuing> {
        let mut resuming = self
            .resuming
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        resuming.insert(session).then(|| Continuing {
            state: Arc::clone(self),
            session,
        })
    }

    /// Open another Run of a Session that already exists.
    /// Record a history row's first durable facts and its continuation, in
    /// one transaction (ADR 0016 D2).
    pub async fn continue_history_session(
        self: &Arc<Self>,
        command: Command,
        session: CorralSessionId,
        run: RunId,
        history: corral_core::BindingKey,
        started: OccurrenceTime,
        at: SystemTime,
    ) -> Result<StartedManagedSession, StateError> {
        self.off_the_reactor(move |store| {
            store.continue_history_session(&command, session, run, history, started, at)
        })
        .await
    }

    pub async fn resume_managed_session(
        self: &Arc<Self>,
        command: Command,
        session: CorralSessionId,
        run: RunId,
        started: OccurrenceTime,
        at: SystemTime,
    ) -> Result<StartedManagedSession, StateError> {
        self.off_the_reactor(move |store| {
            store.resume_managed_session(&command, session, run, started, at)
        })
        .await
    }

    /// A Session's Runs, oldest episode first.
    /// Every Session the registry holds.
    ///
    /// A registry read rather than a runtime one: what is *live here* is the
    /// session list's question, and this one asks what was recorded — which
    /// is how a discovery test, and later a discovery reconciliation, checks
    /// what a delivery actually wrote.
    pub async fn sessions(self: &Arc<Self>) -> Result<Vec<corral_core::Session>, StateError> {
        self.off_the_reactor(Store::sessions).await
    }

    /// The provider runtimes the sweep has found.
    pub fn seen_runtimes(&self) -> &crate::sweep::SharedSeenRuntimes {
        &self.seen_runtimes
    }

    /// The turn an integration operation takes before it writes.
    pub fn integration_turns(&self) -> &crate::integration::WriteTurns {
        &self.integration_turns
    }

    /// The Session an external identity resolves to under any binding kind,
    /// which history enumeration asks before it makes a row (ADR 0016 D2).
    pub async fn session_by_external_id(
        self: &Arc<Self>,
        provider: corral_core::ProviderId,
        external_id: ExternalId,
    ) -> Result<Option<CorralSessionId>, StateError> {
        self.off_the_reactor(move |store| store.session_by_external_id(&provider, &external_id))
            .await
    }

    /// The bindings recorded against one Session.
    pub async fn bindings_of(
        self: &Arc<Self>,
        session: CorralSessionId,
    ) -> Result<Vec<Binding>, StateError> {
        self.off_the_reactor(move |store| store.bindings_of(session))
            .await
    }

    pub async fn runs_of(
        self: &Arc<Self>,
        session: CorralSessionId,
    ) -> Result<Vec<Run>, StateError> {
        self.off_the_reactor(move |store| store.runs_of(session))
            .await
    }

    /// The provider-session binding this Session holds, if it has learned one.
    ///
    /// At most one: binding uniqueness is on `(node, provider, external_id,
    /// kind)`, and a Session that learned two provider identities is the
    /// contest D8 rules on rather than a list to choose from. The first is
    /// returned and a second is reported, because a store holding two is a
    /// fact worth seeing rather than one to average over.
    pub async fn provider_session_binding(
        self: &Arc<Self>,
        session: CorralSessionId,
    ) -> Result<Option<Binding>, StateError> {
        let bindings = self
            .off_the_reactor(move |store| store.bindings_of(session))
            .await?;
        let mut provider_sessions = bindings
            .into_iter()
            .filter(|binding| binding.kind() == BindingKind::ProviderSession);
        let first = provider_sessions.next();
        if provider_sessions.next().is_some() {
            tracing::warn!(%session, "a session holds more than one provider-session binding");
        }
        Ok(first)
    }

    /// Find the Session an external identity names, or mint one for it.
    ///
    /// Binding uniqueness on `(node, provider, external_id, kind)` is what
    /// makes this safe to call from discovery: an identity already known
    /// resolves to the Session that holds it, and nothing duplicates.
    pub async fn resolve_or_create_session(
        self: &Arc<Self>,
        key: BindingKey,
        provenance: Provenance,
        evidence: Evidence,
        at: SystemTime,
    ) -> Result<SessionResolution, StateError> {
        self.off_the_reactor(move |store| {
            store.resolve_or_create_session(key, provenance, evidence, at)
        })
        .await
    }

    /// Record that a Run began against a runtime binding.
    pub async fn record_run_started(
        self: &Arc<Self>,
        run: RunId,
        runtime_binding: corral_core::BindingId,
        occurrence: EvidenceSource,
        started: OccurrenceTime,
    ) -> Result<RecordedRun, StateError> {
        self.off_the_reactor(move |store| {
            store.record_run_started(run, runtime_binding, occurrence, started)
        })
        .await
    }

    /// Record that a Run ended.
    pub async fn record_run_ended(
        self: &Arc<Self>,
        run: RunId,
        end: corral_core::RunEnd,
        at: OccurrenceTime,
    ) -> Result<corral_state::Durability, StateError> {
        self.off_the_reactor(move |store| store.record_run_ended(run, end, at))
            .await
    }

    /// Attach an external identity to a Session Corral already has.
    pub async fn bind(
        self: &Arc<Self>,
        session: CorralSessionId,
        key: BindingKey,
        provenance: Provenance,
        evidence: Evidence,
        at: SystemTime,
    ) -> Result<BindingResolution, StateError> {
        self.off_the_reactor(move |store| store.bind(session, key, provenance, evidence, at))
            .await
    }

    /// What the user chose about a provider's integration, if they chose.
    ///
    /// `None` is not `Disabled`: it says no decision is recorded, and the
    /// caller resolves that against the installed default rather than reading
    /// silence as a refusal (ADR 0013 D6).
    pub async fn integration_intent(
        self: &Arc<Self>,
        provider: ProviderId,
    ) -> Result<Option<RecordedIntent>, StateError> {
        self.off_the_reactor(move |store| store.integration_intent(&provider))
            .await
    }

    pub async fn set_integration_intent(
        self: &Arc<Self>,
        provider: ProviderId,
        intent: IntegrationIntent,
        at: SystemTime,
    ) -> Result<(), StateError> {
        self.off_the_reactor(move |store| store.set_integration_intent(&provider, intent, at))
            .await
    }

    /// Whether an automatic repair may proceed, withdrawing the authority when
    /// the budget is spent.
    pub async fn authorize_repair(
        self: &Arc<Self>,
        fingerprint: RepairFingerprint,
        now: SystemTime,
    ) -> Result<RepairAuthority, StateError> {
        self.off_the_reactor(move |store| {
            store.authorize_repair(&fingerprint, now, policy::REPAIR_BUDGET)
        })
        .await
    }

    /// Record a repair that already succeeded.
    pub async fn record_repair(
        self: &Arc<Self>,
        fingerprint: RepairFingerprint,
        at: SystemTime,
    ) -> Result<(), StateError> {
        self.off_the_reactor(move |store| store.record_repair(&fingerprint, at))
            .await
    }

    /// Re-arm automatic repair after an explicit user reconciliation.
    pub async fn restore_repair_authority(
        self: &Arc<Self>,
        fingerprint: RepairFingerprint,
    ) -> Result<(), StateError> {
        self.off_the_reactor(move |store| store.restore_repair_authority(&fingerprint))
            .await
    }

    /// Replace the evidence supporting a binding.
    pub async fn confirm_binding(
        self: &Arc<Self>,
        binding: corral_core::BindingId,
        evidence: Evidence,
    ) -> Result<Binding, StateError> {
        self.off_the_reactor(move |store| store.confirm_binding(binding, evidence))
            .await
    }

    /// Record that contradictory provider-identity evidence reached a binding.
    pub async fn contest_binding(
        self: &Arc<Self>,
        binding: corral_core::BindingId,
        conflicting: ExternalId,
        evidence: Evidence,
    ) -> Result<Contested, StateError> {
        self.off_the_reactor(move |store| store.contest_binding(binding, conflicting, evidence))
            .await
    }

    /// Confirm the registry can still vouch for durable truth.
    ///
    /// What an answer derived from the registry needs before it may be given.
    /// A session list is answered from the runtime rather than the store, but
    /// it is still a claim made in the store's name — and a mutation must
    /// never be admitted under the condition a read is refused.
    /// Contention is the only refusal this call can produce, and the only one
    /// reported as retryable: `busy` tells a client to send the request again,
    /// and saying that about a refusal nothing diagnosed would be a claim the
    /// daemon cannot make. Anything else is returned as it is, for the caller
    /// to decide — and a refusal still never ends the daemon, because a
    /// refusal leaves the store intact.
    ///
    /// The mapping is this call's, not a shared one: a mutating method's
    /// refusals are mostly permanent, and the phase that serves one writes its
    /// own.
    pub async fn vouch(self: &Arc<Self>) -> Result<Vouched, StateError> {
        match self.off_the_reactor(Store::vouch).await {
            Ok(()) => Ok(Vouched::Yes),
            Err(StateError::Refused(Refusal::Busy { .. })) => Ok(Vouched::NotNow),
            Err(other) => Err(other),
        }
    }

    /// How many managed runs are still running.
    ///
    /// Answered rather than announced: the idle check asks at the moment it
    /// decides, so no caller can forget to report a change and leave a daemon
    /// exiting under live work. Zero when the registry cannot be read, which
    /// only delays an exit rather than causing one.
    pub fn live_sessions(&self) -> usize {
        self.runtime
            .lock()
            .map(|runtime| runtime.sessions.live())
            .unwrap_or(0)
    }

    /// The managed runs this daemon still believes are running.
    ///
    /// For shutdown, which has to be able to name what it is about to end
    /// rather than count it (ADR 0007 L6). Empty when the runtime cannot be
    /// consulted: a shutdown does not stall on a lock, and silence is the
    /// honest report when nothing can be read.
    pub fn running_sessions(&self) -> Vec<crate::runtime::ManagedSession> {
        self.with_runtime(|runtime| {
            runtime
                .sessions
                .describe()
                .into_iter()
                .filter(|session| {
                    session.execution_state == crate::runtime::ExecutionState::Running
                })
                .collect()
        })
        .unwrap_or_default()
    }

    /// Work with the live runtime.
    ///
    /// Synchronous and short: these calls touch in-memory state and message a
    /// session's own thread, so they never wait on a process the way a store
    /// call can wait on a database.
    pub fn with_runtime<T>(&self, work: impl FnOnce(&mut Runtime) -> T) -> Option<T> {
        // A poisoned lock means a holder panicked mid-mutation, which is not
        // something to paper over: the caller answers with what it says when
        // the runtime cannot be consulted rather than reading state nobody
        // finished writing.
        let mut runtime = self.runtime.lock().ok()?;
        Some(work(&mut runtime))
    }

    /// Record that no hook endpoint is serving, so no managed launch may start.
    pub fn hook_endpoint_unavailable(&self) {
        self.hook_endpoint_bound.store(false, Ordering::SeqCst);
    }

    /// Whether this daemon's hook endpoint was bound at startup.
    ///
    /// A startup fact, and named as one. What it rules out is the case worth
    /// ruling out — a daemon that never had an endpoint at all, into which
    /// every managed launch would inject hooks that reach nothing. It does not
    /// rule out an endpoint that stopped being reachable afterwards; the
    /// realistic form of that is the socket being unlinked, which the accept
    /// loop never sees an error for, so noticing it needs a probe this phase
    /// does not have.
    pub fn hook_endpoint_was_bound(&self) -> bool {
        self.hook_endpoint_bound.load(Ordering::SeqCst)
    }

    /// Whether the registry has concluded it can no longer vouch for durable
    /// truth.
    ///
    /// Read from the store itself rather than from anything a connection task
    /// recorded: the conclusion has to survive the task that reached it being
    /// dropped mid-shutdown, or a daemon that stopped over an untrusted store
    /// could still exit as though nothing happened.
    pub fn stopped_vouching(&self) -> bool {
        self.cannot_vouch.load(Ordering::SeqCst)
            // A store that is perfectly healthy and a run lifecycle with a
            // hole in it are the same answer to the only question an exit
            // status can carry: this daemon could not keep its durable state
            // honest (grill Q10).
            || self.observations.integrity() == Integrity::Lost
            || self.lock().stopped_vouching()
    }

    /// Run one store call on the blocking pool.
    async fn off_the_reactor<T: Send + 'static>(
        self: &Arc<Self>,
        work: impl FnOnce(&mut Store) -> Result<T, StateError> + Send + 'static,
    ) -> Result<T, StateError> {
        let state = Arc::clone(self);
        match tokio::task::spawn_blocking(move || work(&mut state.lock())).await {
            Ok(outcome) => outcome,
            // The call did not complete, so nothing can be said about the
            // store — which is the same position as a store that cannot vouch.
            // The store never saw this, so it is recorded here instead; an exit
            // status read from the store alone would report a clean stop.
            Err(source) => {
                self.cannot_vouch.store(true, Ordering::SeqCst);
                Err(StateError::Fatal(FatalState::Storage {
                    detail: format!("a registry call did not complete: {source}"),
                }))
            }
        }
    }

    /// A poisoned lock means another task panicked while holding it. The store
    /// itself decides whether it can still vouch for durable truth, and it
    /// answers every caller the same way once it cannot — so refusing to look
    /// would replace that answer with a stuck daemon.
    fn lock(&self) -> MutexGuard<'_, Store> {
        self.store
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The right to continue one Session, held for as long as the work takes.
///
/// Released by the destructor rather than by every path that could return,
/// because the paths that could return are the ones a continuation fails on —
/// and a claim leaked on a failure would make the Session uncontinuable for
/// the daemon's life.
pub struct Continuing {
    state: Arc<DaemonState>,
    session: CorralSessionId,
}

impl Drop for Continuing {
    fn drop(&mut self) {
        self.state
            .resuming
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&self.session);
    }
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
