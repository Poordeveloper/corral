//! What a delivered hook event becomes.
//!
//! Layer 2 of ADR 0004 D3 in motion: a verified delivery is handed to the
//! provider adapter, which returns normalized facts, and those facts update
//! live evidence and — on the three paths the accepted vocabulary names —
//! the durable log.
//!
//! Ingestion is serial, on one task behind a bounded queue, for two reasons
//! that happen to want the same shape. Two events racing to establish a
//! Session's first provider identity would otherwise both find none and bind
//! two; and the relay must never wait on the store, so the endpoint
//! acknowledges receipt and this runs afterwards. A queue that is full drops
//! the event with diagnostics: at-most-once is the delivery contract, and the
//! evidence model already tolerates missed transitions.

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use corral_core::{
    Assurance, Binding, BindingKey, BindingKind, Evidence, EvidenceSource, ExternalId,
    IdentityStatus, Provenance, ProviderId, RunId,
};
use corral_state::{BindingResolution, Contested, Refusal, StateError};
use tracing::{debug, info, warn};

use crate::provider::{
    AgentFact, AgentFactKind, LaunchScope, LaunchToken, ProviderReport, Uninterpretable,
};
use crate::state::DaemonState;

/// How many delivered events may wait to be interpreted.
///
/// Generous next to how often an agent fires a hook, and bounded because an
/// unbounded queue turns a slow store into unbounded memory. Reaching it means
/// the daemon is already failing to keep up, and the honest answer is a
/// dropped event rather than a growing backlog of stale ones.
const QUEUE: usize = 256;

/// The longest announcing a Run's ending may wait for room in that queue.
///
/// One store wait, because that is what a full queue is waiting on: the
/// consumer's slowest step is a store call, so a slot frees within the store's
/// own wait unless the store is wedged — and waiting past that would only be
/// waiting for something that is not coming.
///
/// This is the recorder's wait, spent in series before it records anything, so
/// `run_lifecycle::LONGEST_OCCURRENCE` adds it to that thread's own budget and
/// the shutdown grace is derived from the sum. Deriving it *from*
/// `LONGEST_RECORD` would make the recorder's worst case twice the bound the
/// shutdown grace was computed from — exactly the falsified derivation the
/// grace exists to prevent.
///
/// Reaching it costs a token that resolves to a finished Run until the daemon
/// exits, which is why it is said out loud rather than shrugged off — but a
/// recorder parked forever behind hook interpretation would stop run endings
/// reaching the log at all, and that is worse.
pub(crate) const RETIREMENT_WAIT: Duration =
    Duration::from_millis(crate::run_lifecycle::STORE_WAIT_MILLIS);

/// How long to leave the queue alone between attempts. Short next to the wait
/// above, which is what actually does the waiting.
const BETWEEN_ATTEMPTS: Duration = Duration::from_millis(20);

/// One delivery that passed the endpoint's checks, waiting to be interpreted.
pub struct Delivered {
    pub token: LaunchToken,
    pub provider: String,
    pub payload: Option<String>,
    pub payload_omitted: Option<String>,
    /// Stamped by the endpoint on arrival. Freshness authority belongs to the
    /// clock of the process that judges freshness (ADR 0004 D3).
    pub observed_at: SystemTime,
    /// The same moment on the monotonic clock, for ordering this daemon's own
    /// observations against each other. A wall clock that steps backwards
    /// would otherwise make every later fact look older than the one on screen.
    pub arrived: Instant,
}

/// Everything the one serial consumer has to act on, in the order it happened.
///
/// A Run ending rides the same queue as the events of that Run, deliberately.
/// Retiring its token on another thread would race the events already waiting
/// here — a session's last `SessionEnd` is delivered and then its process
/// exits, so the two are milliseconds apart — and the retirement would win
/// often enough to lose the tail of every session. One queue makes "after"
/// mean after.
pub(crate) enum Ingest {
    Delivered(Box<Delivered>),
    /// This Run is over. Its token resolves to nothing from here on, whatever
    /// arrives under it (ADR 0004 D5).
    RunEnded(RunId),
}

/// What became of a Run's ending when it was announced.
///
/// Three outcomes and not a boolean, because the caller reports the failure to
/// a person and the two failures are not the same event. A full queue under a
/// live daemon is a daemon falling behind; a queue that is gone is ordinary on
/// the way out and is not worth a warning that reads like a fault.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Retirement {
    /// The queue took it, and the ending is ordered behind this Run's events.
    Taken,
    /// It did not fit inside the bound. The ordering is lost: events of this
    /// Run still waiting will resolve to nothing.
    QueueFull,
    /// There is no consumer. Ordinary during shutdown, and the same answer
    /// when an ingest task ended for any other reason — this side cannot tell
    /// those apart, so it says only what it knows.
    QueueGone,
}

/// Where the endpoint puts what it received, and where a Run's ending is
/// announced.
#[derive(Clone)]
pub struct Deliveries {
    sender: tokio::sync::mpsc::Sender<Ingest>,
    /// How long announcing a Run's ending may wait for room. Carried rather
    /// than read from the constant so a test can drive the give-up path
    /// without spending the real budget on it.
    retirement_wait: Duration,
}

impl Deliveries {
    /// Offer one delivery, or drop it.
    ///
    /// Never waits. The caller is on the path that answers a hook shim, and a
    /// caller that could block there is a caller that can delay the user's
    /// agent.
    pub fn offer(&self, delivered: Delivered) {
        self.send(
            Ingest::Delivered(Box::new(delivered)),
            "a hook event was dropped",
        );
    }

    /// Announce that a Run is over, behind everything it already delivered.
    ///
    /// This one waits, where a delivery does not, and the difference is what
    /// each thing is. A dropped delivery costs one observation, which the
    /// evidence model already tolerates. A dropped retirement leaves a token
    /// resolving to a Run that is over for the daemon's whole life — the exact
    /// state `LaunchTokens::forget_run` exists to prevent — so at-most-once is
    /// the contract for evidence and not for this.
    ///
    /// Waiting is free of the deadlock it looks like: the caller is the run
    /// lifecycle recorder, which already waits on the store, and it announces
    /// an ending *before* it takes the store lock to record one. So a consumer
    /// blocked on that lock cannot be blocked behind this send.
    ///
    /// **Called from outside the async runtime, and only from there.** That is
    /// where its one caller lives — `run_lifecycle` is a thread of its own
    /// precisely so nothing on a reaper's path touches the reactor.
    ///
    /// Anything but `Taken` means the caller retires the token itself: out of
    /// order is a lost tail, and never is a token that can contest the Session
    /// that replaced it. Which of the two it is decides whether that is worth
    /// alarming a reader about, so it is answered rather than flattened.
    #[must_use]
    pub fn run_ended(&self, run: RunId) -> Retirement {
        let deadline = Instant::now() + self.retirement_wait;
        let mut waiting = Ingest::RunEnded(run);
        loop {
            match self.sender.try_send(waiting) {
                Ok(()) => return Retirement::Taken,
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                    // A closed channel usually means the daemon is on its way
                    // out — but it also means an ingest task that ended for
                    // any other reason, and this thread cannot tell them
                    // apart. Reporting success would leave every token of
                    // every later Run resolving for the rest of a daemon that
                    // is still serving. The caller retires it directly, which
                    // costs nothing on the shutdown reading.
                    return Retirement::QueueGone;
                }
                Err(tokio::sync::mpsc::error::TrySendError::Full(returned)) => {
                    if Instant::now() >= deadline {
                        return Retirement::QueueFull;
                    }
                    waiting = returned;
                    std::thread::sleep(BETWEEN_ATTEMPTS);
                }
            }
        }
    }

    fn send(&self, ingest: Ingest, lost: &str) {
        if self.sender.try_send(ingest).is_err() {
            warn!("{lost}: the evidence queue is full or closed");
        }
    }
}

pub(crate) fn queue() -> (Deliveries, tokio::sync::mpsc::Receiver<Ingest>) {
    queue_waiting(RETIREMENT_WAIT)
}

fn queue_waiting(retirement_wait: Duration) -> (Deliveries, tokio::sync::mpsc::Receiver<Ingest>) {
    let (sender, receiver) = tokio::sync::mpsc::channel(QUEUE);
    (
        Deliveries {
            sender,
            retirement_wait,
        },
        receiver,
    )
}

/// Interpret everything the queue carries, one at a time, for as long as the
/// daemon serves.
pub(crate) async fn ingest(
    state: Arc<DaemonState>,
    mut incoming: tokio::sync::mpsc::Receiver<Ingest>,
) {
    while let Some(ingest) = incoming.recv().await {
        match ingest {
            Ingest::Delivered(delivered) => {
                if let Err(error) = ingest_one(&state, *delivered).await {
                    // Never fatal to the daemon. A store that cannot take
                    // evidence costs awareness of one session; the runs it owns
                    // are unaffected, and the store's own conclusions are what
                    // decide whether it can still vouch.
                    warn!(%error, "a hook event could not be recorded");
                }
            }
            Ingest::RunEnded(run) => state.retire_launch_tokens_of(run),
        }
    }
}

async fn ingest_one(state: &Arc<DaemonState>, delivered: Delivered) -> Result<(), StateError> {
    // Resolution first, and it is not authorization: it says which launch this
    // event belongs to. An event with no token, an unknown token, or another
    // launch's token is dropped with diagnostics and never correlated by cwd
    // or time — heuristics never bind (ADR 0004 D5).
    // A token nobody minted and a token whose Run is over reach the same place
    // deliberately: neither is a launch this event may be filed under, and
    // inventing a difference would invite a caller to act on one.
    let Some(scope) = state.resolve_launch_token(&delivered.token) else {
        debug!("a hook event named a launch this daemon does not remember");
        return Ok(());
    };

    // The provider a launch was created as, not the one the message claims. A
    // delivery that disagrees is not this launch's event, whatever else it
    // says.
    if delivered.provider != scope.provider.as_str() {
        warn!(
            claimed = %delivered.provider,
            launched = %scope.provider,
            "a hook event named a provider its launch was not started as",
        );
        return Ok(());
    }

    let Some(payload) = delivered.payload else {
        // The endpoint already logged why it is absent. There is a hook event
        // and no fact to read out of it; the session stays functional and the
        // evidence is simply missing.
        debug!(
            session = %scope.session,
            omitted = delivered.payload_omitted.unwrap_or_else(|| "unstated".to_owned()),
            "a hook event arrived without its payload",
        );
        return Ok(());
    };

    let report = match crate::provider::interpret(scope.provider, &payload) {
        Ok(report) => report,
        Err(Uninterpretable::UnknownEvent) => {
            // Tolerated and counted, asserting nothing — not even the identity
            // it happens to carry (ADR 0004 D3).
            debug!(session = %scope.session, "a hook event named a kind this build has no word for");
            return Ok(());
        }
        Err(Uninterpretable::Malformed) => {
            debug!(session = %scope.session, "a hook payload was not the shape this provider's hooks have");
            return Ok(());
        }
    };

    apply(
        state,
        &scope,
        &report,
        delivered.observed_at,
        delivered.arrived,
    )
    .await
}

async fn apply(
    state: &Arc<DaemonState>,
    scope: &LaunchScope,
    report: &ProviderReport,
    observed_at: SystemTime,
    arrived: Instant,
) -> Result<(), StateError> {
    let session = scope.session;
    let provider = scope.provider;
    let fact = AgentFact {
        kind: report.fact,
        observed_at,
    };
    state.with_runtime(|runtime| runtime.reported.reported(session, provider, fact, arrived));

    let Some(reported_id) = report.identity.clone() else {
        return Ok(());
    };

    // Most reports say nothing new about identity: a turn started, a turn
    // ended, the agent is waiting — each naming the same conversation Corral
    // is already standing behind, in the same Run that already observed it.
    // Those write nothing durable, so asking the store would be a
    // blocking-pool hop and a lock acquisition per prompt, serialized behind
    // this one task, to be told what live state already knew. Live state
    // cannot disagree with the log here: every value in it was published by
    // this same step.
    //
    // What falls through is a report that observes the identity anew, and
    // there are two ways to be one. A provider that reports its own session
    // start says so itself, whenever it says it. A provider that reports no
    // start — Codex reports only completed turns — says it by being the first
    // report of a Run, which is a fact Corral owns because Corral spawned the
    // Run. A rule written around one provider's event name would leave the
    // other's Runs confirming nothing at all (ADR 0004 D7, ADR 0009 D3).
    //
    // The Run half also repairs an inconsistency the second provider exposed:
    // `establish` already refuses to hinge on one event arriving, because
    // delivery is at-most-once, and a confirmation that could only ride a
    // session start was hinging on exactly that.
    //
    // The store is still the authority for everything that *does* write —
    // establishing an identity, confirming one, and contesting one — and every
    // one of those falls through.
    let observed_already = state
        .with_runtime(|runtime| {
            runtime
                .reported
                .identity_observed_in(session, scope.run, &reported_id)
        })
        .unwrap_or(false);
    if observed_already && report.fact != AgentFactKind::SessionStarted {
        return Ok(());
    }

    // A Session whose identity question is closed against this very report is
    // the other case that writes nothing, and it is the one that would
    // otherwise pay most: a contest withdraws the claim for good, and an id
    // another Session holds is never granted to this one — so the same report
    // would make the trip above on every prompt for the rest of the launch's
    // life only to be turned away below. Against this report, not any report:
    // a fresh identity the agent minted after the refusal deserves the store's
    // answer. Live state may answer because this daemon is the one that closed
    // it; a restart forgets it and the store answers again.
    if state
        .with_runtime(|runtime| runtime.reported.identity_closed(session, &reported_id))
        .unwrap_or(false)
    {
        debug!(
            session = %scope.session,
            "a provider identity report on a session whose identity is settled changes nothing",
        );
        return Ok(());
    }

    match state.provider_session_binding(session).await? {
        None => establish(state, scope, reported_id, observed_at).await,
        // Contested is monotonic, and this is where that is enforced rather
        // than described. Later reports of the original id do not restore it,
        // later reports of the conflicting id do not replace it, and a third
        // creates nothing — every one of them is diagnostics from here on
        // (ADR 0004 D8). Without this arm a report of the original id would
        // land in `reobserved`, republish the claim the contest withdrew, and
        // write a fresh confirmation for a binding Corral has said it no
        // longer stands behind.
        Some(existing) if existing.identity_status() == IdentityStatus::Contested => {
            debug!(
                session = %scope.session,
                "a provider identity report on an already contested session changes nothing",
            );
            Ok(())
        }
        Some(existing) if existing.key().external_id() == &reported_id => {
            reobserved(state, scope, &existing, observed_at).await
        }
        Some(existing) => contest(state, scope, &existing, report, reported_id, observed_at).await,
    }
}

/// The first report naming an identity, over a valid token, establishes the
/// Session's provider identity.
///
/// Attested, not Deterministic: live provider-native evidence corroborated by
/// an observed process is the glossary's definition exactly, and Claude minted
/// the id — Corral did not hold it by construction (ADR 0004 D5).
///
/// Any identity-carrying report may establish, not only a session start.
/// Delivery is at-most-once by construction — a full queue drops, a relay past
/// its budget fails open — and a session start fires exactly once per process,
/// so a rule that only it could establish would turn one lost delivery into a
/// Session whose identity is never learned and which can therefore never be
/// continued, with nothing to retry and nothing to tell the person. What makes
/// the claim attributable is the token, and every event of the launch carries
/// the same one; ADR 0004 D7 writes `BindingAdded` "when identity is first
/// learned", which is this moment whichever event brings it.
///
/// The durable *confirmation* stays reserved for a session start
/// (`reobserved`): re-observation is a fresh Run of the same conversation, not
/// every turn of one.
async fn establish(
    state: &Arc<DaemonState>,
    scope: &LaunchScope,
    reported_id: ExternalId,
    observed_at: SystemTime,
) -> Result<(), StateError> {
    let provider = match ProviderId::new(scope.provider.as_str()) {
        Ok(provider) => provider,
        Err(error) => {
            warn!(%error, "a provider name this build declares is not a usable provider id");
            return Ok(());
        }
    };
    let key = BindingKey::new(
        state.node(),
        BindingKind::ProviderSession,
        provider,
        reported_id.clone(),
    );
    // Provenance is `Discovered`, not `CorralCreated`. Corral created the
    // runtime; the conversation identity this edge points at was minted by the
    // provider and learned from its hook. `CorralCreated` would claim Corral
    // named it, which is the same overclaim `Deterministic` would be
    // (ADR 0008 D3).
    let evidence = Evidence::new(
        EvidenceSource::ProviderHook,
        Assurance::Attested,
        observed_at,
    );
    match state
        .bind(
            scope.session,
            key,
            Provenance::Discovered,
            evidence,
            observed_at,
        )
        .await
    {
        Ok(BindingResolution::Created(binding) | BindingResolution::Existing(binding)) => {
            info!(
                session = %scope.session,
                binding = %binding.id(),
                "a managed session's provider identity is attested",
            );
            let session = scope.session;
            let provider = scope.provider;
            let run = scope.run;
            state.with_runtime(|runtime| {
                runtime
                    .reported
                    .identified(session, provider, run, reported_id)
            });
            Ok(())
        }
        // The identity belongs to another Session. Nothing is merged and
        // nothing is guessed: binding uniqueness is what stops one external
        // identity resolving to two Sessions, and the honest outcome is that
        // this Session's identity stays unknown.
        //
        // Recorded live, because nothing in this phase unbinds: every later
        // report from this launch would otherwise cross the blocking pool and
        // take the store lock to be refused the same way, once per prompt for
        // the Session's whole life. Only this refusal is durable in that sense
        // — the others say the store could not take the write now, and a
        // Session is not written off over a "not now".
        Err(StateError::Refused(refusal)) => {
            let claimed = matches!(refusal, Refusal::BindingClaimedByAnotherSession { .. });
            warn!(session = %scope.session, %refusal, "a provider identity was not bound");
            if claimed {
                let session = scope.session;
                state.with_runtime(|runtime| {
                    runtime
                        .reported
                        .identity_claimed_elsewhere(session, reported_id)
                });
            }
            Ok(())
        }
        Err(fatal) => Err(fatal),
    }
}

/// The same identity, observed anew.
///
/// Every report that reaches here is one: the caller's gate has already turned
/// away the re-reports that say nothing — the ones naming an identity this Run
/// has already observed, from a provider that did not call this a start. So
/// the confirmation is written without asking again what kind of report it
/// was.
///
/// Not one per turn, which is the thing this must never become: that would
/// grow the log by a fact for every prompt without recording anything the last
/// one did not.
async fn reobserved(
    state: &Arc<DaemonState>,
    scope: &LaunchScope,
    existing: &Binding,
    observed_at: SystemTime,
) -> Result<(), StateError> {
    let session = scope.session;
    let provider = scope.provider;
    let run = scope.run;
    let external_id = existing.key().external_id().clone();
    state.with_runtime(|runtime| {
        runtime
            .reported
            .identified(session, provider, run, external_id)
    });

    let evidence = Evidence::new(
        EvidenceSource::ProviderHook,
        Assurance::Attested,
        observed_at,
    );
    match state.confirm_binding(existing.id(), evidence).await {
        Ok(_) => Ok(()),
        Err(StateError::Refused(refusal)) => {
            warn!(session = %scope.session, %refusal, "a provider identity was not confirmed");
            Ok(())
        }
        Err(fatal) => Err(fatal),
    }
}

/// A different identity over the same launch.
///
/// Recorded durably, once, and never merged. What it revokes is the authority
/// derived from the identity claim — `session.resume` — and nothing that rides
/// the Deterministic runtime binding: Open, attach, and observation are
/// untouched (ADR 0004 D8).
async fn contest(
    state: &Arc<DaemonState>,
    scope: &LaunchScope,
    existing: &Binding,
    report: &ProviderReport,
    reported_id: ExternalId,
    observed_at: SystemTime,
) -> Result<(), StateError> {
    // Diagnostics, and diagnostics only: a contest is a contest whichever way
    // the runtime came to name a second conversation. What this buys is a
    // person being able to find out *why* continuing this Session stopped
    // being possible — most often because they cleared it.
    let origin = report
        .origin
        .map_or("unstated", crate::provider::SessionOrigin::as_str);
    let evidence = Evidence::new(
        EvidenceSource::ProviderHook,
        Assurance::Attested,
        observed_at,
    );
    let outcome = state
        .contest_binding(existing.id(), reported_id, evidence)
        .await;
    // Withdrawn only where the log says a contest stands. Live state exists to
    // agree with the log, and a refusal means the store recorded nothing: the
    // binding is still Confirmed there, so `session.resume` would go ahead
    // with an identity the row had stopped showing. That combination — hidden
    // from the person, still driving control — is worse than either answer on
    // its own.
    let withdraw = outcome.is_ok();
    match outcome {
        Ok(Contested::Recorded(binding)) => {
            info!(
                session = %scope.session,
                binding = %binding.id(),
                origin,
                "a managed session reported a provider identity that contradicts the one Corral \
                 accepted; continuing this provider session is refused",
            );
        }
        // Already contested. `apply` turns those away before they reach here,
        // so this is the store answering about a race rather than an ordinary
        // path — and the answer is the same either way: contested is
        // monotonic, and a second transition event would record a change that
        // did not happen.
        Ok(Contested::Already(_)) => {
            debug!(
                session = %scope.session,
                origin,
                "a further provider identity report on an already contested session",
            );
        }
        Err(StateError::Refused(refusal)) => {
            warn!(session = %scope.session, %refusal, "a provider identity conflict was not recorded");
        }
        Err(fatal) => return Err(fatal),
    }
    // Recorded and already-recorded both withdraw: the claim is unsafe either
    // way, and a live view that kept publishing it would be the fail-closed
    // behaviour leaking.
    if withdraw {
        let session = scope.session;
        state.with_runtime(|runtime| runtime.reported.contested(session));
    }
    Ok(())
}

#[cfg(test)]
#[path = "hook_evidence_tests.rs"]
mod tests;
