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
    SessionNewParams, SessionNewResult, SessionResumeParams, SessionResumeResult,
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
) -> Result<SessionResumeResult, RequestError> {
    serves_managed_sessions(connection)?;
    let command_id = uuid::Uuid::new_v4().as_hyphenated().to_string();
    connection
        .session_resume(SessionResumeParams {
            command_id,
            session_id: session_id.to_owned(),
        })
        .await
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
