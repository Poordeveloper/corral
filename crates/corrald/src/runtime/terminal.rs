//! The authoritative terminal state for one managed session.
//!
//! `corrald` owns this, not any client (`ARCHITECTURE.md` §3). One bounded
//! emulator per session consumes the PTY's bytes; every surface renders what
//! this holds and none of them keeps a second copy that could disagree.
//!
//! Two consequences are worth naming because they are easy to lose:
//!
//! The emulator answers device queries — DA, DSR, XTVERSION — and those
//! answers must go back to the child even when nobody is attached. An agent
//! that asks what terminal it is talking to and waits for a reply would
//! otherwise stall until a person happened to open a window (ADR 0003 §3).
//!
//! Retention is counted in **bytes**, not rows: the emulator's page model is
//! Ghostty's, where scrollback is a memory budget. A row count would be a
//! different number wearing the same name, and spike S1 recorded that trap
//! after both engines were briefly measured on the wrong axis.

use qwertty_term_vt::stream::{Stream, TerminalHandler};
use qwertty_term_vt::terminal::{Options, Terminal};

use super::spawn::PtyGeometry;

/// How much recent scrollback one session's emulator keeps.
///
/// An initial policy default, not a wire constant (ADR 0003 D7): 4 MiB per
/// session, chosen because M1 has no history backfill, so retention beyond
/// what snapshots carry has no consumer and only costs resident memory in a
/// daemon that holds many sessions.
pub const RETAINED_SCROLLBACK_BYTES: usize = 4 * 1024 * 1024;

/// Bytes the terminal produced in reply to the child's own queries.
///
/// A distinct type from ordinary output because it travels the opposite way —
/// into the PTY, never to a client — and confusing the two would echo a
/// device report onto the screen.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceReply(Vec<u8>);

impl DeviceReply {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// One session's authoritative screen.
pub struct AuthoritativeTerminal {
    stream: Stream<TerminalHandler>,
}

impl AuthoritativeTerminal {
    pub fn new(geometry: PtyGeometry) -> Self {
        let terminal = Terminal::new(Options {
            cols: geometry.cols,
            rows: geometry.rows,
            max_scrollback: RETAINED_SCROLLBACK_BYTES,
            ..Options::default()
        });

        Self {
            stream: Stream::new(TerminalHandler::new(terminal)),
        }
    }

    /// Feed PTY output to the emulator and collect anything the child must be
    /// told in reply.
    ///
    /// The caller writes the reply back to the PTY. Doing it here would make
    /// this type own the descriptor as well as the screen, and the screen is
    /// the part every surface reads.
    #[must_use]
    pub fn consume(&mut self, bytes: &[u8]) -> DeviceReply {
        for byte in bytes {
            self.stream.next(*byte);
        }
        DeviceReply(self.stream.handler.take_output())
    }

    /// Reflow to a new geometry.
    ///
    /// Callers pair this with a new snapshot epoch: replaying pre-resize bytes
    /// into a reflowed screen diverges, which is why resize is an epoch
    /// boundary rather than another delta (ADR 0003, `ARCHITECTURE.md` §3).
    pub fn resize(&mut self, geometry: PtyGeometry) {
        self.stream
            .terminal_mut()
            .resize(geometry.cols, geometry.rows);
    }

    pub fn geometry(&self) -> PtyGeometry {
        let terminal = &self.stream.handler.terminal;
        PtyGeometry {
            rows: terminal.rows,
            cols: terminal.cols,
        }
    }

    /// The window title the child set, if it set one.
    ///
    /// Carried explicitly because the emulator tracks the title but its
    /// serializer does not re-emit it — the one gap S1 found, and the one
    /// ADR 0003 D3 makes Corral's to close when a snapshot is built.
    pub fn title(&self) -> Option<&[u8]> {
        let title = &self.stream.handler.terminal.title;
        (!title.is_empty()).then_some(title.as_slice())
    }

    /// The emulator's own state, for the snapshot serializer and for tests
    /// that assert what the daemon actually holds.
    ///
    /// Read-only on purpose: bytes are the one way the screen changes, so a
    /// caller that could mutate here would be a second writer to the state
    /// this type exists to own.
    pub fn terminal(&self) -> &Terminal {
        &self.stream.handler.terminal
    }
}

impl std::fmt::Debug for AuthoritativeTerminal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthoritativeTerminal")
            .field("geometry", &self.geometry())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "terminal_tests.rs"]
mod tests;
