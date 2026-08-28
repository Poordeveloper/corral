//! What a managed provider launch is, and when a Session may run again.
//!
//! Composition and eligibility, kept apart from the connection that serves
//! them. `connection` reads a request off the wire and answers it; deciding
//! what argv a managed launch gets, what a launch leaves behind when it fails,
//! and whether a continuation may happen at all are one concept with its own
//! rules, and none of them is about a client being connected.

use std::path::PathBuf;
use std::sync::Arc;

use corral_core::{CorralSessionId, NativeResumeEligibility, RunEnd, RunId};

use crate::provider::{
    self, InjectedSettings, InjectionFailed, KnownProvider, LaunchScope, LaunchToken,
};
use crate::runtime::LaunchRequest;
use crate::state::DaemonState;

/// Why a Session cannot be continued right now.
///
/// Every arm is a fact stated to the person, and none of them has an override.
/// M1 offers no `--force`, no "I know it is dead", and no pid heuristic: a
/// second native resume of a provider session that may still be live is two
/// executions driving one conversation (grill Q7).
pub(crate) enum ResumeRefused {
    /// The Session exists and is eligible, but this daemon did not launch it
    /// and so does not know where it ran.
    ///
    /// A known boundary rather than an oversight: where a Run ran is live
    /// state on its handle, and a daemon holding no client and no live Run
    /// exits after its idle grace — so a continuation outlives the provider
    /// process but not the daemon. The plan's "Known limitation" section names
    /// what repairing it needs.
    NotThisDaemon,
    /// The live runtime could not be consulted at all. Not a fact about this
    /// Session — the same request may simply be sent again.
    RuntimeUnavailable,
    IdentityUnknown,
    Eligibility(NativeResumeEligibility),
    UnknownProvider(String),
    RunStillLive,
    EndUnverifiable,
    NoPreviousRun,
    /// Which Run was the most recent episode cannot be established, so which
    /// ending governs cannot be either.
    EpisodeOrderUnknown,
}

/// Decide whether a continuation may happen, before anything is spawned.
///
/// Identity first, then the runtime preconditions. The order is the one the
/// design states, and it matters: a contested Session that has also just been
/// restarted must be refused for the reason that will not go away, not for the
/// one that would.
pub(crate) async fn resume_plan(
    state: &Arc<DaemonState>,
    session: CorralSessionId,
) -> Result<Result<ResumePlan, ResumeRefused>, corral_state::StateError> {
    let Some(binding) = state.provider_session_binding(session).await? else {
        return Ok(Err(ResumeRefused::IdentityUnknown));
    };
    match binding.native_resume_eligibility() {
        NativeResumeEligibility::Eligible => {}
        refused => return Ok(Err(ResumeRefused::Eligibility(refused))),
    }
    let Some(provider) = KnownProvider::from_name(binding.key().provider().as_str()) else {
        return Ok(Err(ResumeRefused::UnknownProvider(
            binding.key().provider().as_str().to_owned(),
        )));
    };

    let runs = state.runs_of(session).await?;
    if runs.iter().any(|run| run.end().is_none()) {
        return Ok(Err(ResumeRefused::RunStillLive));
    }
    // `runs_of` parks a Run whose start the runtime could not state at the end
    // of the list, so its last entry is the most recent episode only while
    // every start is authoritative. That holds of every Run a launch creates
    // and is not something a control decision may assume of a Run some later
    // phase records; reading the wrong episode's end is what would turn an
    // unverifiable ending into a resumable one.
    if runs
        .iter()
        .any(|run| run.started_at().authoritative().is_none())
    {
        return Ok(Err(ResumeRefused::EpisodeOrderUnknown));
    }
    match runs.last().map(corral_core::Run::end) {
        None => return Ok(Err(ResumeRefused::NoPreviousRun)),
        Some(Some(RunEnd::Exited(_))) => {}
        // Unreachable given the live check above, and stated rather than
        // wildcarded so a new end state has to be decided rather than
        // defaulted.
        Some(None) => return Ok(Err(ResumeRefused::RunStillLive)),
        Some(Some(RunEnd::Unverifiable)) => return Ok(Err(ResumeRefused::EndUnverifiable)),
    }

    // Live state, and the last precondition on purpose: a daemon that did not
    // launch this Session does not know where it ran, and a provider resolves
    // which of its own sessions an id names by the directory it was started
    // in. Substituting one would ask for a conversation that is not there.
    // The two `None`s here are different answers and are kept apart. A runtime
    // that could not be consulted is a lock a holder panicked under, which says
    // nothing about this Session; a Session the runtime does not hold is the
    // factual claim below.
    let Some(known) = state.with_runtime(|runtime| {
        runtime
            .sessions
            .get(session)
            .map(|handle| handle.working_directory().to_path_buf())
    }) else {
        return Ok(Err(ResumeRefused::RuntimeUnavailable));
    };
    let Some(working_directory) = known else {
        return Ok(Err(ResumeRefused::NotThisDaemon));
    };

    Ok(Ok(ResumePlan {
        provider,
        external_id: binding.key().external_id().clone(),
        working_directory,
    }))
}

/// Everything a continuation needs, once it is allowed to happen.
pub(crate) struct ResumePlan {
    pub provider: KnownProvider,
    pub external_id: corral_core::ExternalId,
    pub working_directory: PathBuf,
}

impl std::fmt::Display for ResumeRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotThisDaemon => f.write_str(
                "this session was not started by the running Corral daemon, so Corral does not \
                 know where it ran and will not continue it somewhere else",
            ),
            Self::RuntimeUnavailable => {
                f.write_str("Corral could not check this session just now; try again")
            }
            Self::IdentityUnknown => f.write_str(
                "Corral has not learned which provider session this is, so there is nothing to \
                 continue",
            ),
            Self::Eligibility(NativeResumeEligibility::IdentityContested) => f.write_str(
                "this session reported a provider identity that contradicts the one Corral \
                 accepted, so Corral will not continue it",
            ),
            Self::Eligibility(NativeResumeEligibility::AssuranceTooWeak) => f.write_str(
                "Corral is not sure enough which provider session this is to continue it",
            ),
            Self::Eligibility(NativeResumeEligibility::Eligible) => {
                f.write_str("this session can be continued")
            }
            Self::UnknownProvider(name) => write!(
                f,
                "this session belongs to {name}, which this build does not know how to continue"
            ),
            Self::RunStillLive => {
                f.write_str("this session is still running, so there is nothing to continue")
            }
            Self::EndUnverifiable => f.write_str(
                "Corral cannot verify that the previous run has exited, so it will not resume \
                 this provider session automatically",
            ),
            Self::NoPreviousRun => {
                f.write_str("Corral has no record of this session ever having started")
            }
            Self::EpisodeOrderUnknown => f.write_str(
                "Corral cannot establish what this session did most recently, so it will not \
                 resume this provider session automatically",
            ),
        }
    }
}

/// Build the launch of a managed provider session, hook injection included.
///
/// The order is load-bearing. The token is minted and registered *before* the
/// process exists, because a child fires its first hook within milliseconds of
/// starting and a token that became resolvable afterwards would lose the very
/// event identity is learned from. Then the Corral-owned settings file, then
/// the argv that names it.
///
/// One function for both a fresh launch and a continuation, because the two
/// differ in exactly one thing — the arguments — and everything the order
/// above protects is the same for both. `argv` receives the injected file's
/// path and returns the provider's command line.
///
/// The file is written on the blocking pool. Locating the relay stats a path
/// and publishing the settings ends in an `fsync`, which on a loaded or
/// network-backed filesystem is tens to hundreds of milliseconds — and this
/// daemon has one reactor thread. Spending it here would stop every other
/// request, stop the hook endpoint accepting, and push relays past their
/// interference budget, which is the same reason the spawn and every store
/// call are already moved off it.
pub(crate) async fn compose_provider_launch(
    state: &Arc<DaemonState>,
    session: CorralSessionId,
    run: RunId,
    provider: KnownProvider,
    ownership: SessionOwnership,
    working_directory: &std::path::Path,
    argv: impl FnOnce(&std::path::Path) -> Vec<std::ffi::OsString>,
) -> Result<(LaunchRequest, Option<Injected>), String> {
    let scope = LaunchScope {
        session,
        run,
        provider,
    };
    let token = state
        .mint_launch_token(scope)
        .map_err(|_| InjectionFailed::NoRandomness.to_string())?;
    // Recorded with the token, so the first fact to arrive is attributed to
    // the agent Corral started rather than to whatever a payload claims.
    state.with_runtime(|runtime| runtime.reported.launched(session, provider));
    // Nothing below leaves a half-made launch behind: a token that named a
    // process nobody started would keep resolving for the daemon's whole life.
    //
    // The Session is dropped only when this launch is what brought it into
    // being. A continuation names a Session that already exists and already
    // has evidence — its provider, its identity, the last fact it reported —
    // and forgetting that would blank a live row over a launch that failed.
    let forget = move |state: &Arc<DaemonState>| {
        state.forget_launch_token(token);
        if ownership == SessionOwnership::CreatedHere {
            state.with_runtime(|runtime| runtime.reported.forget(session));
        }
    };

    let launch_dir = state.launch_dir().to_path_buf();
    let written = tokio::task::spawn_blocking(move || {
        provider::launch::relay_command(provider, token)
            .and_then(|relay| InjectedSettings::write(&launch_dir, run, provider, &relay))
            .map_err(|failed| failed.to_string())
    })
    .await
    .unwrap_or_else(|_| Err("the launch could not be prepared".to_owned()));
    let settings = match written {
        Ok(settings) => settings,
        Err(failed) => {
            forget(state);
            return Err(failed);
        }
    };

    match LaunchRequest::new(
        provider::program(provider),
        argv(settings.path()),
        working_directory,
    ) {
        Ok(launch) => Ok((
            launch,
            Some(Injected {
                token,
                session,
                run,
                ownership,
            }),
        )),
        Err(refusal) => {
            InjectedSettings::remove_for(state.launch_dir(), run);
            forget(state);
            Err(refusal.to_string())
        }
    }
}

/// Whether a launch is what brought its Session into being.
///
/// The two paths differ in exactly one consequence — what may be undone when
/// the launch fails — and a boolean at the call site would not have said which
/// way round it read.
///
/// Deliberately not called an origin: `provider::SessionOrigin` is the
/// normalized answer to how a *provider* session started, and one crate may
/// not spell two unrelated concepts the same way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionOwnership {
    /// `session.new`: the Session id was minted for this launch and names
    /// nothing yet.
    CreatedHere,
    /// `session.resume`: the Session already exists and already has evidence.
    Preexisting,
}

/// What a launch that may still be abandoned left behind.
pub(crate) struct Injected {
    token: LaunchToken,
    session: CorralSessionId,
    /// The Run whose file this is. Carried rather than passed beside it: the
    /// undo below deletes a live launch's settings file if the two ever
    /// disagree, and a caller that cannot name the wrong Run cannot make that
    /// mistake.
    run: RunId,
    ownership: SessionOwnership,
}

/// Undo a launch that never became one.
///
/// The file is removed here rather than left for the startup sweep, because
/// this is the moment its owner is known not to exist. The token goes with it:
/// it names a Session and Run nothing can ever present.
pub(crate) fn abandon_injection(state: &Arc<DaemonState>, injected: Option<Injected>) {
    let Some(injected) = injected else {
        return;
    };
    InjectedSettings::remove_for(state.launch_dir(), injected.run);
    state.forget_launch_token(injected.token);
    if injected.ownership == SessionOwnership::CreatedHere {
        state.with_runtime(|runtime| runtime.reported.forget(injected.session));
    }
}

#[cfg(test)]
#[path = "managed_launch_tests.rs"]
mod tests;
