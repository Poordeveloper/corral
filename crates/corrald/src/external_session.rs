//! Turning a corroborated external delivery into a Session Corral can see.
//!
//! The claim ladder's top rung (ADR 0014 D3). A token-less delivery carries a
//! provider identity; the ancestry walk says whether a supported provider
//! process was really running when it arrived. Only both together are
//! Attested, and only Attested mints durable facts (D5).
//!
//! Nothing here is a control path. An external Session is read-only by
//! structure: the bindings it produces carry `Provenance::Discovered`, Corral
//! owns no PTY for it, and no operation in this module offers one
//! (ADR 0014 D6).

use std::sync::Arc;
use std::time::SystemTime;

use corral_core::{
    Assurance, BindingKey, BindingKind, Evidence, EvidenceSource, ExitCause, ExternalId,
    OccurrenceTime, Provenance, ProviderId, RunEnd, RunId,
};
use corral_state::{BindingResolution, Refusal, SessionResolution, StateError};
use tracing::{debug, info, warn};

use crate::ancestry::Corroboration;
use crate::platform::process::ProcessIdentity;
use crate::provider::KnownProvider;
use crate::state::DaemonState;

/// What a corroborated delivery produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Discovered {
    /// A Session Corral had never seen, with a Run recorded from the
    /// runtime's own start time.
    Session,
    /// A Session Corral knew, with no Run in progress, now has one on the
    /// runtime that corroborated this delivery.
    Run,
    /// The identity was already known and its Run already in progress: the
    /// second and every later delivery of an external session. Confirmed and
    /// nothing minted.
    AlreadyKnown,
}

/// Record what a corroborated external delivery proves.
///
/// The corroboration is an input rather than something re-derived here: the
/// walk is the platform's business and this is the domain's, and passing the
/// verdict keeps the two testable apart.
pub async fn discovered(
    state: &Arc<DaemonState>,
    provider: KnownProvider,
    identity: ExternalId,
    corroboration: Corroboration,
    observed_at: SystemTime,
) -> Result<Option<Discovered>, StateError> {
    let Corroboration::Reached { process, .. } = corroboration else {
        // Payload identity with no corroboration is honest discovery
        // evidence and nothing more: it proves a provider thread emitted an
        // event, not that the process it names was observed. Promoting it
        // would make a Session out of a claim nothing corroborates — and one
        // measured Codex turn emits a second identity for the provider's own
        // internal work, which is exactly what that would mint (grill Q6′).
        debug!(
            provider = provider.as_str(),
            "an external delivery named an identity nothing corroborates",
        );
        return Ok(None);
    };

    // A runtime this daemon is running itself is not a discovery, whatever
    // its global entry reports. The launch attributes that runtime, through
    // the injected entry and its token, and it may not have yet: the two
    // entries fire milliseconds apart in an unstable order (measured
    // 2026-09-02), and a global delivery taken in first would mint a second
    // Session for the identity and leave the managed Session refused its own
    // for good. Told apart the way the sweep tells them apart — by the
    // process group the daemon created the child as — and withheld when the
    // daemon cannot say which children are its own, because a fact it cannot
    // file under the right Session is not one to guess at.
    match state.with_runtime(|runtime| runtime.owned.groups()) {
        Some(owned) if !owned.contains(&process.group) => {}
        Some(_) => {
            debug!(
                provider = provider.as_str(),
                pid = process.pid,
                "an external delivery corroborated a runtime Corral is running itself",
            );
            return Ok(None);
        }
        None => {
            warn!(
                provider = provider.as_str(),
                "an external delivery could not be told from a managed one; withheld",
            );
            return Ok(None);
        }
    }

    let Ok(named) = ProviderId::new(provider.as_str()) else {
        warn!("a provider name this build declares is not a usable provider id");
        return Ok(None);
    };
    let key = BindingKey::new(
        state.node(),
        BindingKind::ProviderSession,
        named.clone(),
        identity.clone(),
    );
    // Attested: live provider-native evidence — the payload — corroborated by
    // an observed process. Provenance is `Discovered` because the runtime is
    // not Corral's; it was found, not created.
    let evidence = Evidence::new(
        EvidenceSource::ProviderHook,
        Assurance::Attested,
        observed_at,
    );
    let resolution = state
        .resolve_or_create_session(key, Provenance::Discovered, evidence, observed_at)
        .await?;

    match resolution {
        SessionResolution::Created { session, binding } => {
            info!(
                session = %session.id(),
                binding = %binding.id(),
                provider = provider.as_str(),
                "a session running outside Corral is now visible",
            );
            if let RuntimeRecord::Run(run) =
                record_run(state, session.id(), named, &process, observed_at).await?
            {
                shown_under(state, provider, &process, session.id(), identity, run);
            }
            Ok(Some(Discovered::Session))
        }
        // The identity is already bound: the second and every later delivery
        // of an external session. A Run in progress is left alone: a second
        // Run for a runtime that already has one would be a duplicate
        // episode, not a discovery.
        //
        // A known identity is not a completed discovery, though. The Session
        // and its provider binding commit before the runtime and its Run do,
        // and a store that was busy for the second half leaves a Session with
        // no Run — which every later delivery would otherwise confirm and
        // never repair. So the Run is ensured rather than assumed.
        SessionResolution::Existing { session, .. } => {
            if has_live_run(state, session.id()).await? {
                debug!(
                    session = %session.id(),
                    provider = provider.as_str(),
                    "an external delivery confirmed an identity Corral already holds",
                );
                return Ok(Some(Discovered::AlreadyKnown));
            }
            match record_run(state, session.id(), named, &process, observed_at).await? {
                RuntimeRecord::Run(run) => {
                    shown_under(state, provider, &process, session.id(), identity, run);
                    Ok(Some(Discovered::Run))
                }
                RuntimeRecord::Withheld => Ok(Some(Discovered::AlreadyKnown)),
            }
        }
    }
}

/// Put the Session on the live table, in the row its runtime is shown as.
///
/// After the Run is durable and not before: the row a person sees under
/// this Session must have the Run behind it, and a table entry recorded
/// first would name a Session that has nothing in progress if the store
/// refused the second half.
fn shown_under(
    state: &Arc<DaemonState>,
    provider: KnownProvider,
    process: &ProcessIdentity,
    session: corral_core::CorralSessionId,
    external_id: ExternalId,
    run: RunId,
) {
    state.seen_runtimes().identify(
        provider,
        process,
        crate::sweep::Identified {
            session,
            external_id,
            run,
        },
    );
}

/// The runtime an identified Session was in has been seen gone.
///
/// The loss of an observed incarnation is the process table's positive
/// answer, and the Run ends `Exited` with cause `Unknown` — the OS says gone,
/// not why. A store that cannot take the write leaves the Run open and says
/// so; the row is already off the table, and the next restart's
/// re-verification resolves what this pass could not record.
pub async fn runtime_gone(state: &Arc<DaemonState>, identified: &crate::sweep::Identified) {
    let end = RunEnd::Exited(ExitCause::Unknown);
    match state
        .record_run_ended(
            identified.run,
            end,
            OccurrenceTime::FirstObserved(SystemTime::now()),
        )
        .await
    {
        Ok(_) => info!(
            session = %identified.session,
            run = %identified.run,
            ?end,
            "an external run ended with its runtime",
        ),
        Err(error) => warn!(
            %error,
            session = %identified.session,
            run = %identified.run,
            "an external run could not be ended after its runtime went",
        ),
    }
}

/// Whether the Session has a Run with no recorded end.
///
/// A live Run on any runtime binding — managed or discovered — is a Run this
/// delivery must not duplicate. One on a runtime other than the one that
/// corroborated this delivery is the provider carrying a session it already
/// carried somewhere else, which ADR 0014 D7 rules on and this build leaves
/// as it found it.
async fn has_live_run(
    state: &Arc<DaemonState>,
    session: corral_core::CorralSessionId,
) -> Result<bool, StateError> {
    Ok(state
        .runs_of(session)
        .await?
        .iter()
        .any(|run| run.ended_at().is_none()))
}

/// What binding the observed runtime came to.
enum RuntimeRecord {
    /// The runtime is bound to this Session and this Run is recorded on it.
    Run(RunId),
    /// No Run was filed under this Session, and the cause was logged where
    /// it was found.
    Withheld,
}

/// Bind the observed process and record the Run it is already in.
///
/// The Run's start is the runtime's own, not the moment Corral looked: a
/// process that began before this daemon existed still began when it began,
/// and a first-observed instant is never written as a start time
/// (ADR 0002 D6).
async fn record_run(
    state: &Arc<DaemonState>,
    session: corral_core::CorralSessionId,
    provider: ProviderId,
    process: &ProcessIdentity,
    observed_at: SystemTime,
) -> Result<RuntimeRecord, StateError> {
    let Ok(incarnation) = ExternalId::new(incarnation_of(process)) else {
        warn!("an observed process could not be named as an external identity");
        return Ok(RuntimeRecord::Withheld);
    };
    let key = BindingKey::new(state.node(), BindingKind::Runtime, provider, incarnation);
    let evidence = Evidence::new(
        EvidenceSource::NodeRuntimeObservation,
        Assurance::Attested,
        observed_at,
    );
    let binding = match state
        .bind(session, key, Provenance::Discovered, evidence, observed_at)
        .await
    {
        Ok(BindingResolution::Created(binding) | BindingResolution::Existing(binding)) => {
            binding.id()
        }
        // Succession: this runtime is already bound to another Session,
        // because the provider changed which session it carries without the
        // process ending. ADR 0014 D7 rules what that should do — the prior
        // Run ends `SessionChanged`, the successor starts, in one transaction
        // — and this build does not do it yet, so the honest outcome is a
        // visible Session with no Run rather than a Run filed against a
        // runtime that is carrying something else. This one refusal is the
        // tolerated case; every other answer from the store is the store's
        // own, and a fatal one must not read as ordinary succession.
        Err(StateError::Refused(Refusal::BindingClaimedByAnotherSession {
            session: holder,
            ..
        })) => {
            warn!(
                %session,
                %holder,
                "an external runtime is already bound to another session; \
                 succession is not implemented, so this session has no run",
            );
            return Ok(RuntimeRecord::Withheld);
        }
        Err(error) => return Err(error),
    };
    let run = RunId::mint();
    state
        .record_run_started(
            run,
            binding,
            EvidenceSource::NodeRuntimeObservation,
            OccurrenceTime::Authoritative(process.started),
        )
        .await?;
    Ok(RuntimeRecord::Run(run))
}

/// A name for one process incarnation.
///
/// The pid alone is not one: pids are reused, and a binding keyed on a reused
/// pid would file a new process's Run under the Session of the process that
/// held the number before it. The start time is what makes it unique — a
/// reused pid necessarily has a later one — and it is carried at microsecond
/// resolution because that is what the platform reports.
fn incarnation_of(process: &ProcessIdentity) -> String {
    let since_epoch = process
        .started
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_micros())
        .unwrap_or_default();
    format!("pid-{}-{since_epoch}", process.pid)
}

#[cfg(test)]
#[path = "external_session_tests.rs"]
mod tests;

/// Re-verify every external Run this node recorded, on daemon start.
///
/// The no-lying reconciliation law, with external sessions in its scope
/// (ADR 0014 D5). A daemon that starts and finds a Run it recorded before
/// must not leave it shown as running: it either establishes that the process
/// is gone, or reports that it could not, and never quietly keeps a stale
/// claim alive.
///
/// The two answers stay apart. `Gone` is the process table's positive answer
/// and ends the Run `Exited` with cause `Unknown` — the OS says gone, not why.
/// Anything else is `Unverifiable`: a process this account may not inspect, or
/// a platform that cannot look, is not a process that stopped.
pub async fn reverify_external_runs(state: Arc<DaemonState>) {
    let sessions = match state.sessions().await {
        Ok(sessions) => sessions,
        Err(error) => {
            warn!(%error, "external runs could not be re-verified");
            return;
        }
    };
    for session in sessions {
        if let Err(error) = reverify_session(&state, session.id()).await {
            warn!(%error, session = %session.id(), "an external run could not be re-verified");
        }
    }
}

async fn reverify_session(
    state: &Arc<DaemonState>,
    session: corral_core::CorralSessionId,
) -> Result<(), StateError> {
    let bindings = state.bindings_of(session).await?;
    for binding in bindings {
        // Only what discovery recorded. A managed Run is the runtime owner's
        // to reconcile and has its own path, which runs before this one.
        if binding.key().kind() != BindingKind::Runtime
            || binding.provenance() != Provenance::Discovered
        {
            continue;
        }
        let Some(pid) = pid_of(binding.key().external_id().as_str()) else {
            continue;
        };
        for run in state.runs_of(session).await? {
            if run.runtime_binding() != binding.id() || run.ended_at().is_some() {
                continue;
            }
            let observation =
                tokio::task::spawn_blocking(move || crate::platform::process::observe(pid))
                    .await
                    .unwrap_or(crate::platform::process::Observation::Unobservable);
            let end = end_for(&observation, binding.key().external_id().as_str());
            state
                .record_run_ended(
                    run.id(),
                    end,
                    OccurrenceTime::FirstObserved(SystemTime::now()),
                )
                .await?;
            info!(
                session = %session,
                run = %run.id(),
                ?end,
                "an external run was re-verified after a daemon restart",
            );
        }
    }
    Ok(())
}

/// What the process table's answer means for a Run that was live.
fn end_for(observation: &crate::platform::process::Observation, incarnation: &str) -> RunEnd {
    match observation {
        // The same pid running a different incarnation is not this Run's
        // process: the pid was reused, and the process this Run named is gone.
        crate::platform::process::Observation::Identified(process) => {
            if incarnation_of(process) == incarnation {
                // Still there. Nothing ended, and saying otherwise would be
                // the lie this pass exists to prevent — but the Run has to be
                // resolved one way or another on this pass, so an ongoing
                // process is reported as unverifiable rather than closed.
                RunEnd::Unverifiable
            } else {
                RunEnd::Exited(ExitCause::Unknown)
            }
        }
        crate::platform::process::Observation::Gone => RunEnd::Exited(ExitCause::Unknown),
        crate::platform::process::Observation::NotPermitted
        | crate::platform::process::Observation::Unobservable => RunEnd::Unverifiable,
    }
}

/// The pid out of an incarnation name, or nothing when it is not one Corral
/// wrote.
fn pid_of(incarnation: &str) -> Option<u32> {
    incarnation
        .strip_prefix("pid-")?
        .split('-')
        .next()?
        .parse()
        .ok()
}
