//! The loop that carries PTY output into the authoritative terminal.
//!
//! It runs on its own thread because the PTY read is a blocking file
//! descriptor, and it holds the only mutable path to a session's screen. Two
//! rules shape it:
//!
//! Device replies go back to the child immediately, from here. Deferring them
//! to a client would mean an agent that asks its terminal a question waits for
//! a person to attach (`ARCHITECTURE.md` §3).
//!
//! Nothing a subscriber does can slow this loop. A snapshot is minted from the
//! terminal state, so a subscriber that cannot keep up loses its own
//! incremental state and resyncs — it never becomes backpressure on the
//! process producing the output (ADR 0003, grill Q6).
//!
//! The emulator is **not `Send`**: its page model holds raw pointers, so it
//! must be created and used on one thread for its whole life. That is not a
//! limitation to work around with a lock — it is why this is an actor. The
//! thread that reads the PTY owns the screen, and everything else reaches it
//! by message rather than by sharing it.

use std::io::{Read, Write};

use super::spawn::SpawnedRuntime;
use super::terminal::AuthoritativeTerminal;

/// Why the pump stopped reading.
///
/// Both are ordinary. `Closed` is what a child exiting looks like from the
/// master side — on Unix the read fails with EIO, which the backend maps to
/// EOF — so it is an end, not an error.
#[derive(Debug)]
pub enum PumpEnd {
    Closed,
    Failed(std::io::Error),
}

/// Read PTY output into the terminal until the child's side closes.
///
/// Returns when there is nothing left to read. The caller reaps the child and
/// decides what the Run's ending was; this function deliberately makes no
/// claim about why the stream ended, because "the terminal closed" and "the
/// process exited" are different facts.
pub fn pump(
    runtime: &SpawnedRuntime,
    terminal: &mut AuthoritativeTerminal,
) -> Result<PumpEnd, std::io::Error> {
    let mut reader = runtime.reader()?;
    let mut writer = runtime.writer()?;
    // One page of PTY output per read: large enough that a burst is not a
    // syscall per line, small enough that the terminal lock is never held for
    // an unbounded stretch.
    let mut buffer = [0_u8; 8192];

    loop {
        let read = match reader.read(&mut buffer) {
            Ok(0) => return Ok(PumpEnd::Closed),
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Ok(PumpEnd::Failed(error)),
        };

        let reply = terminal.consume(&buffer[..read]);

        if !reply.is_empty() {
            // A child blocked on its own query must not be held behind a
            // client, so this write happens here even though it is the one
            // place the pump talks back to the process.
            writer.write_all(reply.as_bytes())?;
            writer.flush()?;
        }
    }
}

#[cfg(test)]
#[path = "pump_tests.rs"]
mod tests;
