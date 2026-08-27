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
            cwd,
            rows: geometry.map(|geometry| geometry.rows),
            cols: geometry.map(|geometry| geometry.cols),
        })
        .await
}
