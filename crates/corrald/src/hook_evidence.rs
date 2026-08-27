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
use std::time::SystemTime;

use corral_core::{
    Assurance, Binding, BindingKey, BindingKind, Evidence, EvidenceSource, ExternalId, Provenance,
    ProviderId,
};
use corral_state::{BindingResolution, Contested, StateError};
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

/// One delivery that passed the endpoint's checks, waiting to be interpreted.
pub struct Delivered {
    pub token: LaunchToken,
    pub provider: String,
    pub payload: Option<String>,
    pub payload_omitted: Option<String>,
    /// Stamped by the endpoint on arrival. Freshness authority belongs to the
    /// clock of the process that judges freshness (ADR 0004 D3).
    pub observed_at: SystemTime,
}

/// Where the endpoint puts what it received.
#[derive(Clone)]
pub struct Deliveries {
    sender: tokio::sync::mpsc::Sender<Delivered>,
}

impl Deliveries {
    /// Offer one delivery, or drop it.
    ///
    /// Never waits. The caller is on the path that answers a hook shim, and a
    /// caller that could block there is a caller that can delay the user's
    /// agent.
    pub fn offer(&self, delivered: Delivered) {
        if self.sender.try_send(delivered).is_err() {
            warn!("a hook event was dropped: the evidence queue is full or closed");
        }
    }
}

pub fn queue() -> (Deliveries, tokio::sync::mpsc::Receiver<Delivered>) {
    let (sender, receiver) = tokio::sync::mpsc::channel(QUEUE);
    (Deliveries { sender }, receiver)
}

/// Interpret every delivered event, one at a time, for as long as the daemon
/// serves.
pub async fn ingest(
    state: Arc<DaemonState>,
    mut deliveries: tokio::sync::mpsc::Receiver<Delivered>,
) {
    while let Some(delivered) = deliveries.recv().await {
        if let Err(error) = ingest_one(&state, delivered).await {
            // Never fatal to the daemon. A store that cannot take evidence
            // costs awareness of one session; the runs it owns are unaffected,
            // and the store's own conclusions are what decide whether it can
            // still vouch.
            warn!(%error, "a hook event could not be recorded");
        }
    }
}

async fn ingest_one(state: &Arc<DaemonState>, delivered: Delivered) -> Result<(), StateError> {
    // Resolution first, and it is not authorization: it says which launch this
    // event belongs to. An event with no token, an unknown token, or another
    // launch's token is dropped with diagnostics and never correlated by cwd
    // or time — heuristics never bind (ADR 0004 D5).
    // A runtime that cannot be consulted and a token nobody minted reach the
    // same place deliberately: neither is a launch this event can be filed
    // under, and inventing a difference would invite a caller to act on one.
    let resolved = state
        .with_runtime(|runtime| runtime.launch_tokens.resolve(&delivered.token))
        .flatten();
    let Some(scope) = resolved else {
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

    apply(state, &scope, &report, delivered.observed_at).await
}

async fn apply(
    state: &Arc<DaemonState>,
    scope: &LaunchScope,
    report: &ProviderReport,
    observed_at: SystemTime,
) -> Result<(), StateError> {
    if let Some(kind) = report.fact {
        let fact = AgentFact { kind, observed_at };
        let session = scope.session;
        let provider = scope.provider;
        state.with_runtime(|runtime| runtime.reported.reported(session, provider, fact));
    }

    let Some(reported_id) = report.identity.clone() else {
        return Ok(());
    };

    match state.provider_session_binding(scope.session).await? {
        None => establish(state, scope, report, reported_id, observed_at).await,
        Some(existing) if existing.key().external_id() == &reported_id => {
            reobserved(state, scope, &existing, report, observed_at).await
        }
        Some(existing) => contest(state, scope, &existing, reported_id, observed_at).await,
    }
}

/// The first `SessionStart` over a valid token establishes the Session's
/// provider identity.
///
/// Attested, not Deterministic: live provider-native evidence corroborated by
/// an observed process is the glossary's definition exactly, and Claude minted
/// the id — Corral did not hold it by construction (ADR 0004 D5).
///
/// Only a session-start report may establish. A turn fact naming an identity
/// Corral has never accepted is a fact about a conversation, not the moment
/// Corral learned which conversation this is.
async fn establish(
    state: &Arc<DaemonState>,
    scope: &LaunchScope,
    report: &ProviderReport,
    reported_id: ExternalId,
    observed_at: SystemTime,
) -> Result<(), StateError> {
    if report.fact != Some(AgentFactKind::SessionStarted) {
        debug!(
            session = %scope.session,
            "a provider fact named an identity before any session start established one",
        );
        return Ok(());
    }
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
            state.with_runtime(|runtime| {
                runtime.reported.identified(session, provider, reported_id)
            });
            Ok(())
        }
        // The identity belongs to another Session. Nothing is merged and
        // nothing is guessed: binding uniqueness is what stops one external
        // identity resolving to two Sessions, and the honest outcome is that
        // this Session's identity stays unknown.
        Err(StateError::Refused(refusal)) => {
            warn!(session = %scope.session, %refusal, "a provider identity was not bound");
            Ok(())
        }
        Err(fatal) => Err(fatal),
    }
}

/// The same identity, seen again.
///
/// A durable confirmation is written for a session start and nothing else. The
/// re-observation ADR 0004 D7 names is the moment identity is observed anew —
/// a fresh Run of the same conversation — and writing one per turn event would
/// grow the log by one fact for every prompt without recording anything the
/// last one did not.
async fn reobserved(
    state: &Arc<DaemonState>,
    scope: &LaunchScope,
    existing: &Binding,
    report: &ProviderReport,
    observed_at: SystemTime,
) -> Result<(), StateError> {
    let session = scope.session;
    let provider = scope.provider;
    let external_id = existing.key().external_id().clone();
    state.with_runtime(|runtime| runtime.reported.identified(session, provider, external_id));

    if report.fact != Some(AgentFactKind::SessionStarted) {
        return Ok(());
    }
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
    reported_id: ExternalId,
    observed_at: SystemTime,
) -> Result<(), StateError> {
    let evidence = Evidence::new(
        EvidenceSource::ProviderHook,
        Assurance::Attested,
        observed_at,
    );
    let outcome = state
        .contest_binding(existing.id(), reported_id, evidence)
        .await;
    match outcome {
        Ok(Contested::Recorded(binding)) => {
            info!(
                session = %scope.session,
                binding = %binding.id(),
                "a managed session reported a provider identity that contradicts the one Corral \
                 accepted; continuing this provider session is refused",
            );
        }
        // Already contested. Diagnostics only: contested is monotonic, later
        // reports of either id change nothing, and a second transition event
        // would record a change that did not happen.
        Ok(Contested::Already(_)) => {
            debug!(
                session = %scope.session,
                "a further provider identity report on an already contested session",
            );
        }
        Err(StateError::Refused(refusal)) => {
            warn!(session = %scope.session, %refusal, "a provider identity conflict was not recorded");
        }
        Err(fatal) => return Err(fatal),
    }
    // Withdrawn whether this call recorded the contest or found it already
    // recorded: the claim is unsafe either way, and a live view that kept
    // publishing it would be the fail-closed behaviour leaking.
    let session = scope.session;
    state.with_runtime(|runtime| runtime.reported.withdraw_identity(session));
    Ok(())
}

#[cfg(test)]
#[path = "hook_evidence_tests.rs"]
mod tests;
