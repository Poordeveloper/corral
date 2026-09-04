//! The answer to "may this Session be continued, and what must a person be
//! told first" (ADR 0016 D4/D5).
//!
//! `managed_launch::resume_plan` decides whether a continuation *can* happen;
//! this module turns that into what a client shows, in the words a person
//! reads, and binds a disclosure to the exact decision it was made for.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use corral_core::{CorralSessionId, ExternalId, Provenance};
use corral_protocol::ErrorCode;

use crate::history;
use crate::managed_launch::{ResumePlan, ResumeRefused, resume_plan};
use crate::provider::KnownProvider;
use crate::state::DaemonState;

/// What the client must show before continuing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Disclosure {
    pub code: &'static str,
    pub text: String,
}

pub(crate) enum Decision {
    Eligible(ResumePlan),
    EligibleWithDisclosure {
        /// Everything the continuation needs once the disclosure is answered.
        /// A history row's plan is not a `ResumePlan`: no Session, Run, or
        /// binding exists for it yet, and creating them is part of what
        /// continuing it means (ADR 0016 D2).
        plan: HistoryPlan,
        disclosure: Disclosure,
        revision: String,
    },
    Refused {
        /// What `session.resume` answers with. Three answers, because a
        /// client does three different things with them: send it again, ask
        /// a different daemon about the agent, or read what the Session's
        /// own state says. None is `invalid_params` — the parameters were
        /// fine, and a client sent looking for a mistake in its request
        /// would not find one.
        code: ErrorCode,
        reason: String,
    },
}

/// What continuing a history row needs: the identity the provider's store
/// named, and the directory the client asked for.
#[derive(Clone, Debug)]
pub(crate) struct HistoryPlan {
    pub provider: KnownProvider,
    pub external_id: ExternalId,
    pub working_directory: PathBuf,
    /// When Corral read the store and found this session.
    pub observed_at: std::time::SystemTime,
    /// The executable whose version the sealing check read, and the one this
    /// continuation runs. Not the provider's program name: that is resolved
    /// again at exec, and an install upgraded in between would answer it
    /// differently — the version was sealed, so the file it was read from is
    /// what may be launched (ADR 0016 D4).
    pub executable: PathBuf,
}

/// Whether a resume carried the disclosure its decision requires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shown {
    NotNeeded,
    Matching,
    Stale,
}

/// Whose Run has no recorded end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LiveRun {
    Managed,
    External,
}

/// The code of the one disclosure this build can require: a history row's
/// live state is unknown to Corral, and continuing it starts a new Run on a
/// conversation something else may still be using.
pub(crate) const HISTORY_LIVE_STATE_UNKNOWN: &str = "history-live-state-unknown";

/// Why a requested continuation directory cannot be used.
///
/// Each of these is a refusal rather than an occasion to choose a directory:
/// a provider resumes wherever it is started, so picking one for the person
/// would decide, silently, where their agent runs (Q35).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DirectoryRefusal {
    NotSupplied,
    /// A relative path would resolve against whatever working directory this
    /// daemon happens to have, which is the ambient fallback Q35 forbids.
    Relative(PathBuf),
    Missing(PathBuf),
    NotADirectory(PathBuf),
}

impl std::fmt::Display for DirectoryRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotSupplied => f.write_str(
                "Corral needs to be told which directory to continue this session in; it will \
                 not choose one",
            ),
            Self::Relative(path) => write!(
                f,
                "the directory to continue in has to be an absolute path, and {} is not",
                path.display()
            ),
            Self::Missing(path) => write!(f, "{} does not exist", path.display()),
            Self::NotADirectory(path) => write!(f, "{} is not a directory", path.display()),
        }
    }
}

/// The directory a continuation will run in, or why there is none.
///
/// Checked again on the way to a spawn, because a preflight's answer is about
/// the moment it was asked; a directory that has since gone fails the
/// continuation rather than falling back to another one (Q35).
pub(crate) fn usable_directory(requested: Option<&Path>) -> Result<PathBuf, DirectoryRefusal> {
    let Some(requested) = requested.filter(|path| !path.as_os_str().is_empty()) else {
        return Err(DirectoryRefusal::NotSupplied);
    };
    if !requested.is_absolute() {
        return Err(DirectoryRefusal::Relative(requested.to_path_buf()));
    }
    let metadata = std::fs::metadata(requested)
        .map_err(|_| DirectoryRefusal::Missing(requested.to_path_buf()))?;
    if !metadata.is_dir() {
        return Err(DirectoryRefusal::NotADirectory(requested.to_path_buf()));
    }
    Ok(requested.to_path_buf())
}

/// The same question, asked off the reactor.
///
/// `corrald` runs one reactor thread. `metadata` on a stalled mount — NFS, a
/// FUSE filesystem, a volume that went away — holds whichever thread asks for
/// as long as the mount takes to answer, and on that thread it would hold
/// every other client, every hook delivery, and every timer with it. The
/// sealing probe is asked off the reactor for exactly this reason and this is
/// the same filesystem on the same request path.
///
/// `None` is the check not having run at all — the blocking pool is gone,
/// which is what a daemon on its way out looks like. Deliberately not a
/// `DirectoryRefusal`: none of those is true of the directory, and reporting
/// one would tell a person something about their path that nobody looked at.
pub(crate) async fn usable_directory_now(
    requested: Option<PathBuf>,
    check: fn(Option<&Path>) -> Result<PathBuf, DirectoryRefusal>,
) -> Option<Result<PathBuf, DirectoryRefusal>> {
    tokio::task::spawn_blocking(move || check(requested.as_deref()))
        .await
        .ok()
}

/// What a person is told before a history row is continued (ADR 0016 D4).
///
/// Three facts, and no fourth: liveness elsewhere is unknown, another
/// provider process will be started, and this is exactly where.
pub(crate) fn disclosure_text(provider: KnownProvider, directory: &Path) -> String {
    format!(
        "Corral can't tell whether this session is still running somewhere else. Continuing \
         starts another {} process for this session in {}.",
        product_name(provider),
        directory.display()
    )
}

/// Decide, without spawning anything.
///
/// `requested` is the directory the initiating client says it wants the
/// continuation to run in. It governs a history row, which has no location of
/// its own; a Session Corral launched keeps the working directory Corral
/// recorded for it, which the ladder reads from the runtime.
pub(crate) async fn decide(
    state: &Arc<DaemonState>,
    session: CorralSessionId,
    requested: Option<&Path>,
) -> Result<Decision, corral_state::StateError> {
    decide_with(state, session, requested, history::sealed_here).await
}

/// The same, told how to answer whether a provider's layout is sealed here.
///
/// The decision is a parameter for the same reason the enumeration pass takes
/// one: it is the whole of what makes a history row usable, and a test that
/// cannot change it can only exercise this machine's installation.
pub(crate) async fn decide_with(
    state: &Arc<DaemonState>,
    session: CorralSessionId,
    requested: Option<&Path>,
    sealed: fn(KnownProvider) -> Option<history::SealedInstall>,
) -> Result<Decision, corral_state::StateError> {
    let refused = match resume_plan(state, session).await? {
        Ok(plan) => return Ok(Decision::Eligible(plan)),
        Err(refused) => refused,
    };
    if let ResumeRefused::IdentityUnknown = refused
        && let Some(Some(row)) = state.with_runtime(|runtime| runtime.history.row(session).cloned())
    {
        let provider = row.entry.provider;
        // Asked again here, not left to the next enumeration pass. Sealing is
        // not a property of the row: it is what makes the row usable at all,
        // and it is a property of the binary a continuation would launch —
        // which is the one installed now, and can have changed since the pass
        // that read the store. An unmeasured version inherits nothing
        // (ADR 0016), so a row learned under a sealed version is not a licence
        // to start an unmeasured one for the length of a cadence. The working
        // directory is rechecked on this path for the same reason.
        let Some(install) = history::sealed_now(provider, sealed).await else {
            // Just learned, so said once rather than left for the pass: a row
            // this daemon has refused to act on has no business still being
            // listed as one it might.
            state.with_runtime(|runtime| runtime.history.retract(provider));
            return Ok(Decision::Refused {
                code: ErrorCode::SessionNotContinuable,
                reason: format!(
                    "Corral found this session in {}'s history, and the version \
                     installed now is not one Corral has measured, so it cannot \
                     say what continuing it would do.",
                    product_name(provider)
                ),
            });
        };
        let directory =
            match usable_directory_now(requested.map(Path::to_path_buf), usable_directory).await {
                Some(directory) => directory,
                // Transient and about this daemon, not about the request, so it
                // is the one refusal on this rung a client may simply send again.
                None => {
                    return Ok(Decision::Refused {
                        code: ErrorCode::Busy,
                        reason: "Corral could not check this session just now; try again"
                            .to_owned(),
                    });
                }
            };
        return Ok(history_row(session, &row, directory, install.executable));
    }
    let live = match refused {
        ResumeRefused::EndUnverifiable | ResumeRefused::RunStillLive => {
            whose_run_is_open(state, session).await?
        }
        _ => LiveRun::Managed,
    };
    Ok(Decision::Refused {
        code: refused_code(&refused),
        reason: refused_words(&refused, live),
    })
}

/// The fourth rung: no Run is known, so nothing says the conversation is not
/// in use elsewhere. Eligible with that said, in the directory the client
/// asked for — the store holds no location, and both providers resume an id
/// from anywhere and carry on there (ADR 0016, measured), so the directory is
/// Corral's to be told and never to guess (Q35).
fn history_row(
    session: CorralSessionId,
    row: &history::HistoryRow,
    directory: Result<PathBuf, DirectoryRefusal>,
    executable: PathBuf,
) -> Decision {
    let provider = row.entry.provider;
    let directory = match directory {
        Ok(directory) => directory,
        Err(refusal) => {
            return Decision::Refused {
                code: ErrorCode::SessionNotContinuable,
                reason: format!(
                    "Corral found this session in {}'s history, and {refusal}.",
                    product_name(provider)
                ),
            };
        }
    };
    let last_active = row
        .entry
        .last_active
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64);
    Decision::EligibleWithDisclosure {
        disclosure: Disclosure {
            code: HISTORY_LIVE_STATE_UNKNOWN,
            text: disclosure_text(provider, &directory),
        },
        revision: revision(
            session,
            HISTORY_LIVE_STATE_UNKNOWN,
            provider,
            &row.entry.external_id,
            last_active,
            &directory,
        ),
        plan: HistoryPlan {
            provider,
            external_id: row.entry.external_id.clone(),
            working_directory: directory,
            observed_at: row.entry.observed_at,
            executable,
        },
    }
}

async fn whose_run_is_open(
    state: &Arc<DaemonState>,
    session: CorralSessionId,
) -> Result<LiveRun, corral_state::StateError> {
    let runs = state.runs_of(session).await?;
    let Some(open) = runs.iter().find(|run| run.end().is_none()) else {
        return Ok(LiveRun::Managed);
    };
    let bindings = state.bindings_of(session).await?;
    let discovered = bindings.iter().any(|binding| {
        binding.id() == open.runtime_binding() && binding.provenance() == Provenance::Discovered
    });
    Ok(if discovered {
        LiveRun::External
    } else {
        LiveRun::Managed
    })
}

/// A correlation handle for one decision on one set of facts, stable for as
/// long as this daemon process and those facts are: a client that carries it
/// back says "I showed this one". Never a proof a person consented, and never
/// persisted (ADR 0016 D5).
pub(crate) fn revision(
    session: CorralSessionId,
    code: &str,
    provider: KnownProvider,
    external_id: &ExternalId,
    last_active_unix_ms: u64,
    directory: &Path,
) -> String {
    let mut hasher = DefaultHasher::new();
    session.to_string().hash(&mut hasher);
    code.hash(&mut hasher);
    provider.as_str().hash(&mut hasher);
    external_id.as_str().hash(&mut hasher);
    last_active_unix_ms.hash(&mut hasher);
    directory.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Whether a resume carried the disclosure it was required to show.
pub(crate) fn shown(required: Option<&str>, carried: Option<&str>) -> Shown {
    match required {
        None => Shown::NotNeeded,
        Some(required) if carried == Some(required) => Shown::Matching,
        Some(_) => Shown::Stale,
    }
}

fn refused_code(refused: &ResumeRefused) -> ErrorCode {
    match refused {
        ResumeRefused::UnknownProvider(_) => ErrorCode::UnknownProvider,
        ResumeRefused::DirectoryUnknown
        | ResumeRefused::IdentityUnknown
        | ResumeRefused::Eligibility(_)
        | ResumeRefused::RunStillLive
        | ResumeRefused::EndUnverifiable
        | ResumeRefused::NoPreviousRun
        | ResumeRefused::EpisodeOrderUnknown => ErrorCode::SessionNotContinuable,
    }
}

/// The refusal in the person's words, ADR 0016 D4's. The two ends nobody
/// recorded are worded by whose Run it is, because the person does a
/// different thing with each: opens the managed one, waits for the external
/// one. "Still running" is said only of the external Run, whose process the
/// sweep observes for as long as the Run stays open; of the managed one only
/// that its end could not be verified (`AGENTS.md` §Runtime truth).
pub(crate) fn refused_words(refused: &ResumeRefused, live: LiveRun) -> String {
    match (refused, live) {
        (ResumeRefused::RunStillLive, _) => {
            "This session is still running. Open it instead of continuing it.".to_owned()
        }
        (ResumeRefused::EndUnverifiable, LiveRun::External) => {
            "Still running outside Corral. Continuation is unavailable while this session remains \
             live."
                .to_owned()
        }
        (ResumeRefused::EndUnverifiable, LiveRun::Managed) => {
            "Corral couldn't verify that the previous process ended, so continuation is \
             unavailable."
                .to_owned()
        }
        (other, _) => other.to_string(),
    }
}

fn product_name(provider: KnownProvider) -> &'static str {
    match provider {
        KnownProvider::Claude => "Claude Code",
        KnownProvider::Codex => "Codex",
    }
}

#[cfg(test)]
#[path = "continuation_tests.rs"]
mod tests;
