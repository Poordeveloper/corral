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

/// Why a screen stopped being usable.
///
/// One variant, because there is only one way this happens and only one honest
/// response to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Poisoned {
    /// The VT parser panicked on provider output.
    ///
    /// The screen it was building is not in a state anyone may read: a panic
    /// out of a data structure with unsafe internals leaves it half-modified,
    /// so even looking is unsound. Everything about this terminal is refused
    /// from here on.
    ParserPanicked,
}

/// One session's authoritative screen.
pub struct AuthoritativeTerminal {
    stream: Stream<TerminalHandler>,
    poisoned: Option<Poisoned>,
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
            poisoned: None,
        }
    }

    /// Whether this screen may still be read or fed.
    ///
    /// Fail-closed containment, not a repair: Corral never guesses what the
    /// parser meant to do with the bytes that broke it (AGENTS.md §Scope
    /// discipline). Root cause and follow-up:
    /// `docs/evidence/pr3-terminal-fuzz-2026-08-24.md`.
    pub fn poisoned(&self) -> Option<Poisoned> {
        self.poisoned
    }

    /// Feed PTY output to the emulator and collect anything the child must be
    /// told in reply.
    ///
    /// The caller writes the reply back to the PTY. Doing it here would make
    /// this type own the descriptor as well as the screen, and the screen is
    /// the part every surface reads.
    #[must_use]
    pub fn consume(&mut self, bytes: &[u8]) -> DeviceReply {
        if self.poisoned.is_some() {
            return DeviceReply::default();
        }

        // The VT parser is third-party code with a large unsafe surface on the
        // path every untrusted byte takes first (ADR 0003 D1). A panic there
        // is contained rather than allowed to take the daemon's thread with
        // it — but it is never treated as recoverable: the screen is marked
        // and nothing reads it again, because a panic out of a half-modified
        // structure makes even reading unsound.
        let parsed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            for byte in bytes {
                self.stream.next(*byte);
            }
            self.stream.handler.take_output()
        }));

        match parsed {
            Ok(reply) => DeviceReply(reply),
            Err(_) => {
                self.poisoned = Some(Poisoned::ParserPanicked);
                DeviceReply::default()
            }
        }
    }

    /// Reflow to a new geometry.
    ///
    /// Callers pair this with a new snapshot epoch: replaying pre-resize bytes
    /// into a reflowed screen diverges, which is why resize is an epoch
    /// boundary rather than another delta (ADR 0003, `ARCHITECTURE.md` §3).
    pub fn resize(&mut self, geometry: PtyGeometry) {
        if self.poisoned.is_some() {
            return;
        }
        self.stream
            .terminal_mut()
            .resize(geometry.cols, geometry.rows);
    }

    /// The screen's size, or `None` once the screen may no longer be read.
    ///
    /// Every reader goes through an `Option` rather than one of them getting a
    /// plain value: a poisoned screen has no size anyone may state, and a
    /// caller that could skip the check would be the one that reads a
    /// half-modified structure.
    pub fn geometry(&self) -> Option<PtyGeometry> {
        if self.poisoned.is_some() {
            return None;
        }
        let terminal = &self.stream.handler.terminal;
        Some(PtyGeometry {
            rows: terminal.rows,
            cols: terminal.cols,
        })
    }

    /// The window title the child set, if it set one.
    ///
    /// Carried explicitly because the emulator tracks the title but its
    /// serializer does not re-emit it — the one gap S1 found, and the one
    /// ADR 0003 D3 makes Corral's to close when a snapshot is built.
    pub fn title(&self) -> Option<&[u8]> {
        if self.poisoned.is_some() {
            return None;
        }
        let title = &self.stream.handler.terminal.title;
        (!title.is_empty()).then_some(title.as_slice())
    }

    /// The emulator's own state, for the snapshot serializer and for tests
    /// that assert what the daemon actually holds.
    ///
    /// Read-only on purpose: bytes are the one way the screen changes, so a
    /// caller that could mutate here would be a second writer to the state
    /// this type exists to own.
    pub fn terminal(&self) -> Option<&Terminal> {
        self.poisoned
            .is_none()
            .then_some(&self.stream.handler.terminal)
    }
}

impl std::fmt::Debug for AuthoritativeTerminal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthoritativeTerminal")
            .field("geometry", &self.geometry())
            .field("poisoned", &self.poisoned)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "terminal_tests.rs"]
mod tests;
