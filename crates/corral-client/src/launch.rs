//! Starting and continuing a session: the one request every surface makes.
//!
//! `corral new`, the list's `new` and the Desktop's New Session differ in
//! what they do with the session afterwards, not in how they ask for one.
//! Asked three times, the surfaces would drift on the parts a person never
//! sees — the size the session is born at, the directory it starts in,
//! whether an id is minted per attempt — and the divergence would show up as
//! surfaces starting subtly different sessions from the same words.
//!
//! Nothing here is the provider grammar's authority. `corrald` revalidates
//! every `session.new` against ADR 0012 whatever a client checked, and no
//! client's acceptance is ever the safety boundary (PR9 plan, round 2 Q8):
//! what a surface may do in advance is refuse what the daemon's hello already
//! says it cannot serve, and render the daemon's refusal in its own words.

use std::path::{Path, PathBuf};

use corral_protocol::method::{
    self, SessionContinuationParams, SessionNewParams, SessionNewResult, SessionResumeParams,
    SessionResumeResult,
};
use corral_protocol::{ErrorCode, ProtocolError};

use crate::{Connection, RequestError};

/// What a person asked to start.
///
/// Two ways, and they stay apart all the way down to the wire: a provider name
/// and a program name are different namespaces, and a surface that collapsed
/// them would make `corral new bash` mean whichever the daemon guessed
/// (grill Q6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Requested {
    /// An agent Corral composes the command for.
    Provider { name: String, args: Vec<String> },
    /// The raw runtime harness: a command the person composed.
    Command(Vec<String>),
}

/// Read what a person typed as one of the two things it can be.
///
/// The separator is what tells them apart, and it is the same separator the
/// command line uses: `claude` is an agent, `-- bash` is a command. Nothing is
/// guessed — an unknown agent name is a request the daemon refuses by name,
/// which is what keeps the two namespaces distinct (grill Q6).
///
/// `None` for words that name neither: an empty line, or a separator with
/// nothing after it.
#[must_use]
pub fn requested(words: &[String]) -> Option<Requested> {
    let (first, rest) = words.split_first()?;
    if first == SEPARATOR {
        return (!rest.is_empty()).then(|| Requested::Command(rest.to_vec()));
    }
    // An optional separator before the agent's own arguments, so the same line
    // works whether or not a person types it — and so an argument that looks
    // like a separator cannot start a second list.
    let args = match rest.split_first() {
        Some((next, after)) if next == SEPARATOR => after,
        _ => rest,
    };
    Some(Requested::Provider {
        name: first.clone(),
        args: args.to_vec(),
    })
}

/// What separates Corral's own words from the ones it passes through.
pub const SEPARATOR: &str = "--";

/// Where a session is born: facts about where the person asked from, which
/// only a surface knows.
///
/// A terminal surface reads both off the terminal it is at; a graphical one
/// asks. Either way they are preferences the daemon reconciles, never part of
/// what the command means (`SessionNewParams`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LaunchSite {
    /// The directory the session starts in. Absent lets the daemon supply
    /// one; present is never silently replaced.
    pub working_directory: Option<PathBuf>,
    /// The size the first attaching client wants, when it has one.
    pub rows: Option<u16>,
    pub cols: Option<u16>,
}

/// Ask `corrald` to start a session.
pub async fn start_session(
    connection: &mut Connection,
    requested: Requested,
    site: LaunchSite,
) -> Result<SessionNewResult, RequestError> {
    let (provider, args, argv) = match requested {
        Requested::Provider { name, args } => {
            serves_managed_sessions(connection)?;
            (Some(name), args, Vec::new())
        }
        Requested::Command(argv) => (None, Vec::new(), argv),
    };

    // Minted per invocation, and the same id is what a retry would carry: it
    // is what stops a lost response from starting a second agent. No surface
    // retries yet, so nothing here re-sends it — the id is the daemon's
    // protection against a client that does (ADR 0002, Q13).
    let command_id = uuid::Uuid::new_v4().as_hyphenated().to_string();

    // Unbounded on purpose, unlike the questions a surface asks about state
    // that already exists. Starting a session builds a PTY and spawns a child,
    // which on a loaded machine can take longer than any patience worth
    // having — and a client that gave up would report a failure for a session
    // the daemon went on to create. A caller that cannot afford to wait bounds
    // it itself, where it can say what it will do with the session that
    // arrives anyway.
    connection
        .session_new(SessionNewParams {
            command_id,
            argv,
            provider,
            args,
            cwd: site
                .working_directory
                .map(|path| path.to_string_lossy().into_owned()),
            rows: site.rows,
            cols: site.cols,
        })
        .await
}

/// Ask `corrald` to continue a session as a new Run.
///
/// No geometry and no directory: a continuation runs where the Session already
/// ran, and Corral resolves that from what it recorded rather than from where
/// the person happens to be standing.
pub async fn continue_session(
    connection: &mut Connection,
    session_id: &str,
    shown: Shown,
    working_directory: Option<&Path>,
    show: &mut dyn FnMut(&str),
) -> Result<Continued, RequestError> {
    serves_managed_sessions(connection)?;
    if !serves(connection, corral_protocol::capability::HISTORY_SESSIONS) {
        return resume_without_preflight(connection, session_id).await;
    }
    let working_directory = working_directory.map(|path| path.to_string_lossy().into_owned());
    let decision = connection
        .session_continuation(SessionContinuationParams {
            session_id: session_id.to_owned(),
            working_directory: working_directory.clone(),
        })
        .await?;
    let disclosure_revision = match decision.decision.as_str() {
        method::CONTINUATION_ELIGIBLE => None,
        method::CONTINUATION_ELIGIBLE_WITH_DISCLOSURE => {
            let (Some(disclosure), Some(revision)) =
                (decision.disclosure, decision.disclosure_revision)
            else {
                return Err(RequestError::Protocol {
                    detail: "the daemon required a disclosure and sent none".to_owned(),
                });
            };
            match shown {
                // Before the continuation, not after it. Answering in advance
                // is answering a question the person is owed the text of;
                // printing it once the provider is already running says what
                // happened, which is not a disclosure (ADR 0016 D5).
                Shown::InAdvance => {
                    show(&disclosure.text);
                    Some(revision)
                }
                // The decision moved between being shown and being answered
                // — a directory changed, a Run appeared — so what the person
                // said yes to is not what would happen. Asked again rather
                // than carried over (ADR 0016 D5).
                Shown::Accepted(seen) if seen == revision => Some(revision),
                Shown::NotYet | Shown::Accepted(_) => {
                    return Ok(Continued::NeedsDisclosure {
                        text: disclosure.text,
                        revision,
                    });
                }
            }
        }
        // `refused`, and any decision this build has no word for: acting on
        // an unknown decision would be acting on a guess.
        _ => {
            // The daemon's own code where it sent one. A `busy` refusal is
            // one the person may simply send again, and answering every
            // refusal as `session_not_continuable` tells them the opposite.
            // Absent means a daemon that predates the field, not a permanent
            // refusal — but there is nothing better to say than the general
            // case (`AGENTS.md` §Protocol).
            let code = decision
                .code
                .map_or(ErrorCode::SessionNotContinuable, ErrorCode::from);
            return Err(RequestError::Refused(ProtocolError::new(
                code,
                decision
                    .reason
                    .unwrap_or_else(|| "Corral will not continue this session".to_owned()),
            )));
        }
    };
    let command_id = uuid::Uuid::new_v4().as_hyphenated().to_string();
    connection
        .session_resume(SessionResumeParams {
            command_id,
            session_id: session_id.to_owned(),
            disclosure_revision,
            // The directory the decision above was made on. Sending a
            // different one is a different decision, and the daemon says so
            // rather than starting a process somewhere nobody was shown.
            working_directory,
        })
        .await
        .map(|started| Continued::Started { started })
}

/// The directory this client asks a continuation to run in: its own working
/// directory, which is client policy and never something the daemon supplies
/// for it (Q35). `None` when the process cannot name its own, which refuses
/// a continuation that needs one.
#[must_use]
pub fn working_directory() -> Option<PathBuf> {
    std::env::current_dir().ok()
}

/// Whether the person has already been shown, and answered, whatever the
/// daemon requires disclosing before this continuation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shown {
    /// Ask first: a required disclosure comes back as
    /// [`Continued::NeedsDisclosure`] for the surface to show.
    NotYet,
    /// The person answered yes in advance (`corral continue --yes`), so
    /// whatever the preflight requires is shown and accepted at once.
    InAdvance,
    /// The person was shown this exact revision and said yes to it.
    Accepted(String),
}

/// What continuing produced.
#[derive(Clone, Debug)]
pub enum Continued {
    Started {
        started: SessionResumeResult,
    },
    /// The daemon requires this be shown first. Carry `revision` back with
    /// [`Shown::Accepted`] once it has been.
    NeedsDisclosure {
        text: String,
        revision: String,
    },
}

/// Continue against a daemon that predates the preflight (ADR 0016).
///
/// The same request this client sent before `session.continuation` existed,
/// and not a degraded form of the current one: such a daemon enumerates no
/// history rows, so nothing it can continue has a disclosure to show, and the
/// requested directory governs the history rung alone — a Session Corral
/// launched runs where Corral recorded it, which is the answer both builds
/// give. Sending the newer fields anyway would have them dropped as unknown,
/// which is the silent substitution the directory rule exists to prevent
/// (Q35); refusing instead would take a working command away from someone who
/// upgraded one half of a pair.
async fn resume_without_preflight(
    connection: &mut Connection,
    session_id: &str,
) -> Result<Continued, RequestError> {
    let command_id = uuid::Uuid::new_v4().as_hyphenated().to_string();
    connection
        .session_resume(SessionResumeParams {
            command_id,
            session_id: session_id.to_owned(),
            disclosure_revision: None,
            working_directory: None,
        })
        .await
        .map(|started| Continued::Started { started })
}

/// Refuse before asking when the daemon does not serve managed agents.
///
/// `session.resume` and a provider-named `session.new` are additive, so the
/// protocol version says nothing about them and an older daemon answers with
/// `method_not_found` or "this needs a command" — both of which send a person
/// looking for a mistake in what they typed. The capability is what the hello
/// carries the answer in, and asking it is what turns that into a fact about
/// the daemon.
fn serves_managed_sessions(connection: &Connection) -> Result<(), RequestError> {
    if serves(connection, corral_protocol::capability::MANAGED_SESSIONS) {
        return Ok(());
    }
    Err(RequestError::Refused(ProtocolError::new(
        ErrorCode::MethodNotFound,
        "the running Corral daemon does not start or continue agent sessions; it is older than \
         this build",
    )))
}

/// Whether the daemon named this contract in its hello.
///
/// The one question a surface asks before offering an action: an action the
/// daemon cannot serve is absent, not disabled, and never attempted on the
/// chance that it might work (`AGENTS.md` §Protocol).
#[must_use]
pub fn serves(connection: &Connection, capability: &str) -> bool {
    connection.peer().capabilities.contains(capability)
}

#[cfg(test)]
#[path = "launch_tests.rs"]
mod tests;
