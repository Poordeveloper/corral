//! Starting a session: the one request both surfaces make.
//!
//! `corral new` and the list's `new` differ in what they do with the session
//! afterwards, not in how they ask for one. Asked twice, the two would drift
//! on the parts a person never sees — the size the session is born at, the
//! directory it starts in, whether an id is minted per attempt — and the
//! divergence would show up as two surfaces starting subtly different
//! sessions from the same words.

use corral_client::{Connection, RequestError};
use corral_protocol::method::{SessionNewParams, SessionNewResult};

use crate::attach::Geometry;

/// Ask `corrald` to start a session running `argv`.
///
/// Born at this terminal's size and in this process's directory: both are
/// facts about where the person asked from, which only a surface knows.
pub async fn start_session(
    connection: &mut Connection,
    argv: Vec<String>,
) -> Result<SessionNewResult, RequestError> {
    let geometry = Geometry::of(&std::io::stdin());
    let cwd = std::env::current_dir()
        .ok()
        .map(|path| path.to_string_lossy().into_owned());

    // Minted per invocation, and the same id is what a retry would carry: it
    // is what stops a lost response from starting a second agent. No surface
    // retries yet, so nothing here re-sends it — the id is the daemon's
    // protection against a client that does (ADR 0002, Q13).
    let command_id = uuid::Uuid::new_v4().as_hyphenated().to_string();

    let asked = connection.session_new(SessionNewParams {
        command_id,
        argv,
        cwd,
        rows: geometry.map(|geometry| geometry.rows),
        cols: geometry.map(|geometry| geometry.cols),
    });

    match tokio::time::timeout(crate::ANSWER, asked).await {
        Ok(started) => started,
        // Bounded like every wait in this crate: the surface that asked is
        // holding a terminal in raw mode, and a daemon that never answers must
        // not leave a person there.
        Err(_) => Err(RequestError::Protocol {
            detail: format!("nothing within {} seconds", crate::ANSWER.as_secs()),
        }),
    }
}
