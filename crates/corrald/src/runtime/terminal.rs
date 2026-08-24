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
    /// Taken and forgotten the moment the screen is poisoned, never dropped.
    ///
    /// `PageList::drop` walks the packed page list calling `Box::from_raw` on
    /// every node, and a panic out of that layer leaves the list mid-mutation
    /// — a node linked but not initialised, or already moved to the free list.
    /// Refusing to *read* a half-modified structure while still letting its
    /// destructor walk it is not containment: it is the same unsound
    /// traversal, run later and unconditionally.
    ///
    /// So a poisoned emulator is leaked on purpose. One session's retained
    /// scrollback is a bounded, one-off cost; undefined behaviour in a daemon
    /// that is still serving every other session is not.
    screen: Option<Stream<TerminalHandler>>,
    poisoned: Option<Poisoned>,
}

impl AuthoritativeTerminal {
    pub fn new(geometry: PtyGeometry) -> Self {
        let terminal = Terminal::new(Options {
            cols: geometry.cols(),
            rows: geometry.rows(),
            max_scrollback: RETAINED_SCROLLBACK_BYTES,
            ..Options::default()
        });

        Self {
            screen: Some(Stream::new(TerminalHandler::new(terminal))),
            poisoned: None,
        }
    }

    /// Whether this screen may still be read or fed.
    ///
    /// Root cause and follow-up:
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
        self.contain(|this| {
            let screen = this.screen.as_mut()?;
            for byte in bytes {
                screen.next(*byte);
            }
            Some(DeviceReply(screen.handler.take_output()))
        })
        .flatten()
        .unwrap_or_default()
    }

    /// Reflow to a new geometry.
    ///
    /// Callers pair this with a new snapshot epoch: replaying pre-resize bytes
    /// into a reflowed screen diverges, which is why resize is an epoch
    /// boundary rather than another delta (ADR 0003, `ARCHITECTURE.md` §3).
    pub fn resize(&mut self, geometry: PtyGeometry) {
        let _ = self.contain(|this| {
            let screen = this.screen.as_mut()?;
            screen
                .terminal_mut()
                .resize(geometry.cols(), geometry.rows());
            Some(())
        });
    }

    /// Serialize this screen, against the default budget.
    pub fn snapshot(
        &mut self,
    ) -> Result<super::snapshot::Snapshot, super::snapshot::SnapshotError> {
        self.snapshot_within(super::snapshot::SnapshotBudget::DEFAULT)
    }

    /// Serialize this screen against an explicit budget.
    ///
    /// Here rather than beside the serializer because serialization walks the
    /// same packed pages that parsing writes, so it is one of the three ways
    /// into the structure this type owns the poison flag for — and the flag is
    /// what a contained panic has to be able to set (ADR 0007 L5).
    pub fn snapshot_within(
        &mut self,
        budget: super::snapshot::SnapshotBudget,
    ) -> Result<super::snapshot::Snapshot, super::snapshot::SnapshotError> {
        match self.contain(|this| super::snapshot::encode_within(this, budget)) {
            Some(snapshot) => snapshot,
            // Asked again rather than answered here: the serializer already
            // owns what a poisoned screen is refused with, and a second copy
            // of that sentence is a second thing to keep true.
            None => super::snapshot::encode_within(self, budget),
        }
    }

    /// Enter the emulator, or refuse to.
    ///
    /// The one door into the emulator's packed pages — parsing, reflow, and
    /// serialization all walk them, so a boundary around only the first would
    /// give three identical risks different answers. `None` means the screen
    /// was already poisoned or has just become so; a poisoned screen is never
    /// read again, because a panic out of a half-modified page makes even
    /// reading unsound.
    ///
    /// Fail-closed containment, not a repair: Corral never guesses what the
    /// parser meant to do with the bytes that broke it (AGENTS.md §Scope
    /// discipline).
    fn contain<T>(&mut self, work: impl FnOnce(&mut Self) -> T) -> Option<T> {
        self.screen.as_ref()?;
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(self))) {
            Ok(outcome) => Some(outcome),
            Err(_) => {
                self.poison();
                None
            }
        }
    }

    /// Give up this screen without running its destructor.
    ///
    /// The one place poisoning happens, so the reason and the disposal cannot
    /// drift apart: the field's documentation carries why the broken emulator
    /// is forgotten rather than dropped.
    fn poison(&mut self) {
        self.poisoned = Some(Poisoned::ParserPanicked);
        if let Some(screen) = self.screen.take() {
            std::mem::forget(screen);
        }
    }

    /// The screen's size, or `None` once the screen may no longer be read.
    ///
    /// Every reader goes through an `Option` rather than one of them getting a
    /// plain value: a poisoned screen has no size anyone may state, and a
    /// caller that could skip the check would be the one that reads a
    /// half-modified structure.
    pub fn geometry(&self) -> Option<PtyGeometry> {
        let terminal = &self.screen.as_ref()?.handler.terminal;
        // The emulator's own size, which came from a validated geometry and
        // cannot have become invalid since.
        Some(PtyGeometry::expect_valid(terminal.rows, terminal.cols))
    }

    /// The window title the child set, if it set one.
    ///
    /// Carried explicitly because the emulator tracks the title but its
    /// serializer does not re-emit it — the one gap S1 found, and the one
    /// ADR 0003 D3 makes Corral's to close when a snapshot is built.
    pub fn title(&self) -> Option<&[u8]> {
        let title = &self.screen.as_ref()?.handler.terminal.title;
        (!title.is_empty()).then_some(title.as_slice())
    }

    /// The emulator's own state, for the snapshot serializer and for tests
    /// that assert what the daemon actually holds.
    ///
    /// Read-only on purpose: bytes are the one way the screen changes, so a
    /// caller that could mutate here would be a second writer to the state
    /// this type exists to own.
    pub fn terminal(&self) -> Option<&Terminal> {
        Some(&self.screen.as_ref()?.handler.terminal)
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
