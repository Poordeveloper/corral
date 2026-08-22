//! The Unix process-model boundary.
//!
//! `corrald` never daemonizes by forking. A client that auto-starts it spawns
//! a fresh child, and that child detaches itself here before any runtime
//! exists, so the daemon's whole life is one process with one lifecycle.

use tracing::debug;

/// Leave the spawning terminal's session.
///
/// Failure is not a daemon failure: `setsid` refuses when the caller already
/// leads a process group, which simply means there is nothing to detach from.
pub fn detach_session() {
    match rustix::process::setsid() {
        Ok(session) => debug!(session = session.as_raw_nonzero().get(), "detached"),
        Err(errno) => debug!(%errno, "already detached from the spawning session"),
    }
}
