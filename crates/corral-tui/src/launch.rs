//! Starting a session: the one request both surfaces make.
//!
//! `corral new` and the list's `new` differ in what they do with the session
//! afterwards, not in how they ask for one. Asked twice, the two would drift
//! on the parts a person never sees — the size the session is born at, the
//! directory it starts in, whether an id is minted per attempt — and the
//! divergence would show up as two surfaces starting subtly different
//! sessions from the same words.

use corral_client::{Connection, RequestError};
use corral_protocol::method::{
    self, SessionContinuationParams, SessionNewParams, SessionNewResult, SessionResumeParams,
    SessionResumeResult,
};
use corral_protocol::{ErrorCode, ProtocolError};

use crate::attach::Geometry;

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

/// Ask `corrald` to start a session.
///
/// Born at this terminal's size and in this process's directory: both are
/// facts about where the person asked from, which only a surface knows.
pub async fn start_session(
    connection: &mut Connection,
    requested: Requested,
) -> Result<SessionNewResult, RequestError> {
    let geometry = Geometry::of(std::io::stdin());
    let cwd = std::env::current_dir()
        .ok()
        .map(|path| path.to_string_lossy().into_owned());
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

    // Unbounded on purpose, unlike the questions this crate asks about state
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
            cwd,
            rows: geometry.map(|geometry| geometry.rows),
            cols: geometry.map(|geometry| geometry.cols),
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
    working_directory: Option<&std::path::Path>,
) -> Result<Continued, RequestError> {
    serves_managed_sessions(connection)?;
    let working_directory = working_directory.map(|path| path.to_string_lossy().into_owned());
    let decision = connection
        .session_continuation(SessionContinuationParams {
            session_id: session_id.to_owned(),
            working_directory: working_directory.clone(),
        })
        .await?;
    let (disclosure_revision, disclosed) = match decision.decision.as_str() {
        method::CONTINUATION_ELIGIBLE => (None, None),
        method::CONTINUATION_ELIGIBLE_WITH_DISCLOSURE => {
            let (Some(disclosure), Some(revision)) =
                (decision.disclosure, decision.disclosure_revision)
            else {
                return Err(RequestError::Protocol {
                    detail: "the daemon required a disclosure and sent none".to_owned(),
                });
            };
            match shown {
                Shown::InAdvance => (Some(revision), Some(disclosure.text)),
                // The decision moved between being shown and being answered
                // — a directory changed, a Run appeared — so what the person
                // said yes to is not what would happen. Asked again rather
                // than carried over (ADR 0016 D5).
                Shown::Accepted(seen) if seen == revision => (Some(revision), None),
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
            return Err(RequestError::Refused(ProtocolError::new(
                ErrorCode::SessionNotContinuable,
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
        .map(|started| Continued::Started { started, disclosed })
}

/// The directory this client asks a continuation to run in: its own working
/// directory, which is client policy and never something the daemon supplies
/// for it (Q35). `None` when the process cannot name its own, which refuses
/// a continuation that needs one.
#[must_use]
pub fn working_directory() -> Option<std::path::PathBuf> {
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
        /// The disclosure the daemon required, when the person answered it
        /// in advance: a `--yes` still renders what it said yes to (ADR
        /// 0016 D5).
        disclosed: Option<String>,
    },
    /// The daemon requires this be shown first. Carry `revision` back with
    /// [`Shown::Accepted`] once it has been.
    NeedsDisclosure { text: String, revision: String },
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
    if connection
        .peer()
        .capabilities
        .contains(corral_protocol::capability::MANAGED_SESSIONS)
    {
        return Ok(());
    }
    Err(RequestError::Refused(ProtocolError::new(
        ErrorCode::MethodNotFound,
        "the running Corral daemon does not start or continue agent sessions; it is older than \
         this build",
    )))
}
