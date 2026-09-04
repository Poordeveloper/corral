//! What a managed provider launch is, and when a Session may run again.
//!
//! Composition and eligibility, kept apart from the connection that serves
//! them. `connection` reads a request off the wire and answers it; deciding
//! what argv a managed launch gets, what a launch leaves behind when it fails,
//! and whether a continuation may happen at all are one concept with its own
//! rules, and none of them is about a client being connected.

use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use corral_core::{CorralSessionId, NativeResumeEligibility, OccurrenceTime, RunEnd, RunId};
use corral_state::{StartedManagedSession, StateError};
use tracing::error;

use crate::provider::{
    self, InjectedSettings, InjectionFailed, KnownProvider, LaunchIntent, LaunchScope, LaunchToken,
    RelayInvocation,
};
use crate::runtime::{ExecutionState, LaunchRequest, PendingSession, PtyGeometry};
use crate::state::DaemonState;

/// What became of a managed launch, in the vocabulary of what happened rather
/// than of what a client is told.
///
/// The dispatcher maps these onto wire answers; nothing here knows about error
/// codes. Both `session.new` and `session.resume` come through here, because
/// the part they share is the part that must never diverge: who kills the
/// child when the store refuses, and who ends a runtime the registry would not
/// take.
pub(crate) enum Committed {
    /// A Run started, is registered, and is being served.
    Started {
        session: CorralSessionId,
        run: RunId,
    },
    /// This command had already executed. Whatever it made is what the caller
    /// is told about; nothing new is running.
    Replayed {
        session: CorralSessionId,
        run: RunId,
    },
    /// Nothing was spawned, so no Run exists to report.
    NotSpawned(String),
    /// The child ran and the store would not record it. It has been reaped.
    StoreRefused(StateError),
}

/// Spawn a composed launch, commit its Run, and serve it — or leave nothing
/// behind.
///
/// The one copy of the ladder both entry points walk. Every failure past the
/// spawn has to hang up a child that is already running: a process left alive
/// with no durable Run is unlistable for the daemon's life, and a second copy
/// of that reasoning is a place for one of them to be repaired alone.
pub(crate) async fn spawn_and_commit<Commit, Committing>(
    state: &Arc<DaemonState>,
    session: CorralSessionId,
    run: RunId,
    launch: LaunchRequest,
    geometry: PtyGeometry,
    injected: Option<Injected>,
    commit: Commit,
) -> Committed
where
    Commit: FnOnce(corral_state::LaunchedRun, SystemTime) -> Committing,
    Committing: Future<Output = Result<StartedManagedSession, StateError>>,
{
    let program = std::path::PathBuf::from(launch.program());
    // Taken before the request is spent. Every Corral launch records where it
    // ran, and building the episode here rather than at each call site is
    // what makes that true of all three of them rather than of whichever ones
    // remembered (Q35).
    let working_directory = launch.working_directory().to_path_buf();
    let pending = match spawn_off_the_reactor(launch, geometry).await {
        Ok(pending) => pending,
        // The command never ran, so no Run exists to report. Saying otherwise
        // would record a runtime occurrence that never happened.
        Err(error) => {
            abandon_injection(state, injected);
            return Committed::NotSpawned(error);
        }
    };
    // A provider launch gets its screen read against the provider's manifest,
    // and its version bound at the launch boundary — from the installation the
    // program resolved to, only if that metadata predates the process
    // (grill Q12). A raw command has neither.
    let provider = injected
        .as_ref()
        .and_then(|injected| state.resolve_launch_token(&injected.token))
        .map(|scope| scope.provider);
    // The version is read before the manifest is attached, because it is half
    // of what seals a reading: a rule measured on one build asserts nothing
    // about the build actually drawing this screen.
    let version = provider.and_then(|provider| {
        let began = pending.began();
        crate::provider::version::resolve_program(&program)
            .and_then(|resolved| crate::provider::version::installed_version(provider, &resolved))
            .and_then(|installed| installed.bound_to(began))
    });
    if provider.is_some() {
        let version = version.clone();
        state.with_runtime(|runtime| runtime.reported.versioned(session, version));
    }
    let pending = match provider.and_then(|provider| state.manifest_for(provider)) {
        Some(manifest) => pending.detect_with(manifest, version),
        None => pending,
    };
    // Known as Corral's from this moment, not from the serve below: the
    // process is on the table now, and a sweep that reads it while the store
    // is still committing must not list it as a runtime outside Corral. A
    // runtime lock this cannot take is met again at the serve, which ends the
    // child rather than keep one it cannot register.
    let owned = pending.owned_child();
    let _ = state.with_runtime(|runtime| runtime.owned.register(owned));

    // A concrete runtime occurrence now exists, so its start may be written —
    // and must be, before anything that could report its end exists. The
    // producer of `RunEnded` is created only after this commits (grill Q9).
    //
    // Two instants, because they answer different questions: when the runtime
    // began, which Corral watched, and when Corral accepted the command. ADR
    // 0002 D6 keeps them apart, and one value used for both would be the
    // conflation it exists to prevent. The first is the spawn's own, measured
    // where the process was created rather than here — the gap across a
    // blocking-pool hop and a reschedule is arbitrary under load, and an
    // instant measured after the fact is not an authoritative one.
    let began = pending.began();
    let launched = corral_state::LaunchedRun {
        run,
        started: OccurrenceTime::Authoritative(began),
        working_directory,
    };
    let started = match commit(launched, SystemTime::now()).await {
        Ok(started) => started,
        Err(error) => {
            // The child is running and its Run is not a durable fact. It is
            // hung up and reaped here rather than left alive and unlistable,
            // and no ending is reported: with no durable start there is no Run
            // to end (grill Q9).
            abandon(pending);
            abandon_injection(state, injected);
            return Committed::StoreRefused(error);
        }
    };

    // Only a receipt this call wrote describes the runtime this call spawned.
    // A replay here would mean another execution already committed — which the
    // claim held by both callers makes impossible on one daemon, and which must
    // still never leave a second process running.
    if !started.executed() {
        abandon(pending);
        abandon_injection(state, injected);
        return Committed::Replayed {
            session: started.session(),
            run: started.run(),
        };
    }

    // For a continuation, the prior Run's final screen is superseded by this
    // one's live screen: one Session shows one runtime, and the record it
    // replaces is the episode that ended (ADR 0007 L1).
    let handle = pending.serve(session, run, state.observations().clone());

    // The child is already running by now, so the handle must not simply be
    // dropped if the runtime registry cannot take it: the reader thread holds
    // another sender, so dropping this one would leave a live process and its
    // screen running unreachable for the daemon's lifetime.
    // Held outside the closure so a lock the daemon could not take does not
    // drop it: the closure would never run and the handle would go with it.
    let mut orphan = Some(handle);
    let stored = state.with_runtime(|runtime| {
        if let Some(handle) = orphan.take() {
            runtime.sessions.insert(handle);
        }
    });
    if stored.is_none() {
        if let Some(orphaned) = orphan.take() {
            // Its ending is still reported and still recorded: the Run is a
            // durable fact now, and a session Corral gives up on is an episode
            // that ends rather than one that stays open forever.
            orphaned.shut_down();
        }
        // The caller still answers `Started`. Not `busy`, which invites a
        // retry: the command has already executed and its receipt is durable,
        // so a retry would replay this same answer rather than do anything
        // different. What the caller is told is what happened — a Run that
        // started and is ending — and the session it names is the one the log
        // holds.
        error!(%session, %run, "a managed run could not be registered and was ended");
    }

    Committed::Started { session, run }
}

/// Spawn on the blocking pool.
///
/// `openpty` plus fork and exec can take a while under memory pressure, and
/// `LaunchRequest::new` stats the working directory. On the daemon's one
/// reactor thread that window is one where nothing else is served — the same
/// cost every other call here goes out of its way to avoid.
async fn spawn_off_the_reactor(
    launch: LaunchRequest,
    geometry: PtyGeometry,
) -> Result<PendingSession, String> {
    tokio::task::spawn_blocking(move || {
        crate::runtime::spawn_session(&launch, geometry).map_err(|error| error.to_string())
    })
    .await
    .unwrap_or_else(|_| Err("the session could not be started".to_owned()))
}

/// End a runtime whose Run never became a durable fact.
///
/// A plain thread rather than the blocking pool, and deliberately not waited
/// on. Reaping is the one thing a child can make Corral wait for indefinitely
/// — a process may ignore a hang-up and never read its terminal — and neither
/// a client's request nor the daemon's own exit may be held by that. The
/// blocking pool would hold the exit: dropping the tokio runtime waits for
/// every blocking task that has started.
fn abandon(pending: PendingSession) {
    std::thread::spawn(move || pending.abandon());
}

/// Why a Session cannot be continued right now.
///
/// Every arm is a fact stated to the person, and none of them has an override.
/// M1 offers no `--force`, no "I know it is dead", and no pid heuristic: a
/// second native resume of a provider session that may still be live is two
/// executions driving one conversation (grill Q7).
pub(crate) enum ResumeRefused {
    /// The Session is eligible, and Corral cannot say where its last Run ran.
    ///
    /// Every Run Corral launches records its directory, so this is a Run
    /// Corral *found*: where a discovered process runs is knowable from the
    /// OS and is not something this phase looks at. Refused rather than
    /// guessed, because a provider resolves which of its sessions an id names
    /// by where it is started (Q35).
    DirectoryUnknown,
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
        // A Run with no recorded end is *running* only where this daemon is
        // the one running it. Everywhere else nothing supports the claim — a
        // reaper that never got to write an end leaves the same absence — and
        // telling a person "this session is still running" from an absent
        // record is the unknown-means-running inference, in the direction that
        // reaches them (`AGENTS.md` §Runtime truth).
        let running_here = state
            .with_runtime(|runtime| {
                runtime
                    .sessions
                    .get(session)
                    .is_some_and(|handle| handle.execution_state() == ExecutionState::Running)
            })
            .unwrap_or(false);
        return Ok(Err(if running_here {
            ResumeRefused::RunStillLive
        } else {
            ResumeRefused::EndUnverifiable
        }));
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
        // Unreachable given the check above, which answered every Run without
        // a recorded end. Stated rather than wildcarded so a new end state has
        // to be decided rather than defaulted, and answered the way that check
        // answers when it cannot see the process itself.
        Some(None) => return Ok(Err(ResumeRefused::EndUnverifiable)),
        Some(Some(RunEnd::Unverifiable)) => return Ok(Err(ResumeRefused::EndUnverifiable)),
    }

    // Read from the Run, not from a runtime handle: where a Run ran is a fact
    // about that episode, and the daemon that held the handle is gone by the
    // time this matters. A provider resolves which of its own sessions an id
    // names by the directory it was started in, so an absent one is refused
    // rather than substituted (Q35).
    let Some(working_directory) = runs
        .last()
        .and_then(|run| run.working_directory().map(std::path::Path::to_path_buf))
    else {
        return Ok(Err(ResumeRefused::DirectoryUnknown));
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
            Self::DirectoryUnknown => f.write_str(
                "Corral did not start this session and does not know which directory it ran in, \
                 so it will not continue it somewhere else",
            ),
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
/// differ in exactly one thing — the intent — and everything the order above
/// protects is the same for both. What that intent composes into is the
/// provider's, including whether it needs anything written at all: this knows
/// only that a launch may leave an artifact behind and that an artifact that
/// exists has to be undone (ADR 0009 D1).
///
/// Composition runs on the blocking pool. Locating the relay stats a path, and
/// a provider that publishes a settings file ends in an `fsync`, which on a
/// loaded or network-backed filesystem is tens to hundreds of milliseconds —
/// and this daemon has one reactor thread. Spending it here would stop every
/// other request, stop the hook endpoint accepting, and push relays past their
/// interference budget, which is the same reason the spawn and every store
/// call are already moved off it.
pub(crate) struct LaunchTarget<'a> {
    pub provider: KnownProvider,
    /// The executable to run. A continuation of a history row names the file
    /// the sealing check read the version from; a launch that makes no version
    /// claim names the provider's program and lets exec resolve it
    /// (ADR 0016 D4).
    pub program: &'a std::ffi::OsStr,
    pub working_directory: &'a std::path::Path,
    pub intent: LaunchIntent,
}

pub(crate) async fn compose_provider_launch(
    state: &Arc<DaemonState>,
    session: CorralSessionId,
    run: RunId,
    ownership: SessionOwnership,
    target: LaunchTarget<'_>,
) -> Result<(LaunchRequest, Option<Injected>), String> {
    let LaunchTarget {
        provider,
        program,
        working_directory,
        intent,
    } = target;
    // Asked before anything is minted or written. A managed session's whole
    // point is that it reports; if nothing is listening for what it reports,
    // starting it produces a session that looks managed and can never be
    // continued — the same outcome an unusable relay binary is refused over
    // (`provider::launch::usable_relay`). Raw sessions are unaffected: they
    // never claimed to report.
    if !state.hook_endpoint_was_bound() {
        return Err(
            "Corral cannot receive what an agent reports right now, so it will not start a \
             session it could not watch"
                .to_owned(),
        );
    }

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
    let composed = tokio::task::spawn_blocking(move || {
        RelayInvocation::compose(provider, token)
            .and_then(|relay| provider::compose_launch(provider, &intent, &relay, &launch_dir, run))
            .map_err(|failed| failed.to_string())
    })
    .await
    .unwrap_or_else(|_| Err("the launch could not be prepared".to_owned()));
    let composed = match composed {
        Ok(composed) => composed,
        Err(failed) => {
            forget(state);
            return Err(failed);
        }
    };

    // The caller's, not this function's: a continuation of a history row may
    // only run the executable whose version the sealing check read, while a
    // launch that makes no version claim resolves the provider's program name
    // the way any command does (ADR 0016 D4).
    match LaunchRequest::new(program, composed.argv, working_directory) {
        Ok(launch) => Ok((
            launch,
            Some(Injected {
                token,
                session,
                ownership,
                artifact: composed.artifact,
            }),
        )),
        Err(refusal) => {
            // The token first. What it authorises is correlation of evidence
            // to a Run, and this is the moment that Run stops existing; the
            // file it named, if this provider took one, is an artifact nobody
            // reads after startup.
            forget(state);
            if let Some(artifact) = &composed.artifact {
                provider::launch::removed_without_waiting(artifact);
            }
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
    ownership: SessionOwnership,
    /// What this launch wrote, when its provider writes anything.
    ///
    /// The artifact itself rather than the Run that names it: the undo below
    /// deletes exactly the file this launch published, so a caller that cannot
    /// name another launch's file cannot delete one. `None` is a provider
    /// whose injection rides its argv, not a launch that failed to write —
    /// there is nothing on disk to undo, and a lifecycle that ran anyway would
    /// be deleting a file it never created (ADR 0009 D1).
    artifact: Option<InjectedSettings>,
}

/// Undo a launch that never became one.
///
/// The file, where there is one, is removed here rather than left for the
/// startup sweep, because this is the moment its owner is known not to exist.
/// The token goes with it: it names a Session and Run nothing can ever
/// present.
pub(crate) fn abandon_injection(state: &Arc<DaemonState>, injected: Option<Injected>) {
    let Some(injected) = injected else {
        return;
    };
    // The token stops resolving before anything else, and before this function
    // can yield. A caller reaching here on the store-refusal path has already
    // spawned a child and hung it up without waiting — a process may take its
    // time dying, and one that fires a hook on the way out must not find a
    // token still naming a Run the daemon has decided never happened.
    state.forget_launch_token(injected.token);
    if injected.ownership == SessionOwnership::CreatedHere {
        state.with_runtime(|runtime| runtime.reported.forget(injected.session));
    }
    if let Some(artifact) = &injected.artifact {
        provider::launch::removed_without_waiting(artifact);
    }
}

#[cfg(test)]
#[path = "managed_launch_tests.rs"]
mod tests;
