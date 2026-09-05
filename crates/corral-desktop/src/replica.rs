//! The Desktop's replica of a session's terminal, rebuilt from the wire.
//!
//! ADR 0003 fixed the wire as a snapshot at a position plus sequenced deltas;
//! ADR 0017 added the prefix a replica needs to rebuild from a snapshot alone:
//! its geometry, and a palette checkpoint when the connection's differs. This
//! is the client half of that contract — what is held, what is installed,
//! what is discarded, and when a fresh screen is asked for — kept apart from
//! the window so it can be proved without one.
//!
//! The emulator is qwertty-term-vt, the engine corrald renders with (spike
//! scenario 1): whatever the daemon's screen can express, this one reproduces
//! cell for cell. Its parser runs inside a poison boundary: a panic in it
//! destroys this replica and never the process (spike grill Q3).

use std::panic::{AssertUnwindSafe, catch_unwind};

use corral_protocol::terminal::{Epoch, FrameKind, Sequence, TerminalFrame};
use qwertty_term_vt::modes::Mode;
use qwertty_term_vt::snapshot::SnapshotWindow;
use qwertty_term_vt::stream::{Stream, TerminalHandler};
use qwertty_term_vt::terminal::{Options, Terminal};

/// Rows and columns, as `Geometry` and `Resize` carry them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub rows: u16,
    pub cols: u16,
}

impl Geometry {
    /// The four big-endian bytes both frames use.
    #[must_use]
    pub fn decode(payload: &[u8]) -> Option<Self> {
        let bytes: [u8; 4] = payload.try_into().ok()?;
        Some(Self {
            rows: u16::from_be_bytes([bytes[0], bytes[1]]),
            cols: u16::from_be_bytes([bytes[2], bytes[3]]),
        })
    }

    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        let mut payload = Vec::with_capacity(4);
        payload.extend_from_slice(&self.rows.to_be_bytes());
        payload.extend_from_slice(&self.cols.to_be_bytes());
        payload
    }
}

/// What the daemon's hello promised about the frames around a snapshot
/// (ADR 0017 D5). A frame the daemon did not promise says nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Promised {
    pub geometry: bool,
    pub palette: bool,
}

/// The input modes the replica is in, from which keys are encoded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modes {
    /// DECCKM: cursor keys send `ESC O x` rather than `ESC [ x`.
    pub cursor_keys: bool,
    /// Pasted text is wrapped in `ESC [ 200 ~` and `ESC [ 201 ~`.
    pub bracketed_paste: bool,
}

/// Why there is no screen to show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Absence {
    /// No snapshot has been installed for the current state: the first one is
    /// on its way, or a fresh one was asked for.
    AwaitingSnapshot,
    /// Under a daemon that sends no `Geometry`, no size is known to build a
    /// replica at until this client's own cell grid has been established and
    /// sent (round 2, Q13). Never a guessed 80×24.
    AwaitingGrid,
    /// The daemon's screen could not be installed, or the parser failed on its
    /// bytes, and this episode's one automatic recovery is spent. A new epoch
    /// from the daemon, or opening the session again, starts a fresh one.
    Unavailable,
}

impl Absence {
    /// What the person is told, in the words `PRODUCT.md` §4 allows: never a
    /// claim about the agent or the process, only about this screen.
    #[must_use]
    pub fn line(self) -> &'static str {
        match self {
            Self::AwaitingSnapshot | Self::AwaitingGrid => "Waiting for the screen…",
            Self::Unavailable => "Terminal unavailable",
        }
    }
}

/// What applying a frame asks of the owner.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Applied {
    /// The screen changed.
    pub redraw: bool,
    /// Send a `ResyncRequest`: what this replica holds is no longer trusted.
    pub resync: bool,
    /// The daemon refused something, in its words.
    pub refusal: Option<String>,
}

/// The replica for one attachment.
pub struct Replica {
    promised: Promised,
    /// The size this client last asked for. Under a daemon that sends no
    /// `Geometry` it is the only size there is, and it is never presented as
    /// daemon-confirmed (Q13).
    requested: Option<Geometry>,
    /// A reshape this client asked for and has not yet seen a new epoch for.
    /// The epoch it produces is this client's doing, and a recovery attempt
    /// cannot manufacture the event that re-arms its own budget (round 1, #4).
    own_reshape_pending: bool,
    /// The epoch of the installed screen, or of the last one that was.
    epoch: Option<Epoch>,
    screen: Result<Screen, Absence>,
    held_geometry: Option<Held<Geometry>>,
    held_palette: Option<Held<Vec<u8>>>,
    /// Whether this episode's one automatic resync has been spent.
    recovery_spent: bool,
}

struct Screen {
    stream: Stream<TerminalHandler>,
    geometry: Geometry,
}

/// A prefix member, held until the snapshot it describes.
struct Held<T> {
    epoch: Epoch,
    sequence: Sequence,
    value: T,
}

impl<T> Held<T> {
    fn describes(&self, frame: &TerminalFrame) -> bool {
        self.epoch == frame.epoch && self.sequence == frame.sequence
    }
}

impl Replica {
    #[must_use]
    pub fn new(promised: Promised) -> Self {
        Self {
            promised,
            requested: None,
            own_reshape_pending: false,
            epoch: None,
            screen: Err(if promised.geometry {
                Absence::AwaitingSnapshot
            } else {
                Absence::AwaitingGrid
            }),
            held_geometry: None,
            held_palette: None,
            recovery_spent: false,
        }
    }

    /// The grid this client established locally and is asking the daemon for.
    ///
    /// Under a daemon that sends no `Geometry`, the first grid is what the
    /// first snapshot is built at, so a fresh snapshot is asked for once a
    /// size exists: the one the daemon already sent was built for nobody.
    pub fn requested(&mut self, geometry: Geometry) -> Applied {
        let first_under_legacy = !self.promised.geometry && self.requested.is_none();
        self.requested = Some(geometry);
        self.own_reshape_pending = true;
        Applied {
            resync: first_under_legacy,
            ..Applied::default()
        }
    }

    /// Apply one frame the daemon sent, in the order it sent them.
    pub fn apply(&mut self, frame: &TerminalFrame) -> Applied {
        match frame.kind {
            FrameKind::Geometry => self.hold_geometry(frame),
            FrameKind::Palette => self.hold_palette(frame),
            FrameKind::Snapshot => self.install(frame),
            FrameKind::Delta => self.feed_delta(frame),
            FrameKind::ChannelError => Applied {
                refusal: Some(String::from_utf8_lossy(&frame.payload).into_owned()),
                ..Applied::default()
            },
            // Kinds only a client sends, and kinds this build does not know.
            // Both are skipped: the length prefix already said how much to
            // drop.
            FrameKind::Input | FrameKind::Resize | FrameKind::ResyncRequest => Applied::default(),
            // The skippability rule has one owner, in the protocol crate, so
            // a receiver added later cannot quietly decide it differently.
            other if other.is_skippable() => Applied::default(),
            other => {
                debug_assert!(false, "no rule for {other:?}");
                Applied::default()
            }
        }
    }

    /// The screen, or why there is none.
    pub fn screen(&self) -> Result<&Terminal, Absence> {
        match &self.screen {
            Ok(screen) => Ok(&screen.stream.handler.terminal),
            Err(absence) => Err(*absence),
        }
    }

    /// The visible rows, cursor and palette, or why there are none.
    pub fn window(&self) -> Result<SnapshotWindow, Absence> {
        self.screen().map(|terminal| terminal.snapshot_window(0))
    }

    /// The size of the installed screen: what the daemon minted the snapshot
    /// for, or under an old daemon what this client asked for.
    #[must_use]
    pub fn geometry(&self) -> Option<Geometry> {
        self.screen.as_ref().ok().map(|screen| screen.geometry)
    }

    /// The epoch outgoing frames are labelled with: the installed screen's,
    /// or the first before anything arrived.
    #[must_use]
    pub fn epoch(&self) -> Epoch {
        self.epoch.unwrap_or(Epoch(0))
    }

    /// The modes keys are encoded under. Defaults until a screen exists.
    #[must_use]
    pub fn modes(&self) -> Modes {
        self.screen().map_or(Modes::default(), |terminal| Modes {
            cursor_keys: terminal.modes.get(Mode::CursorKeys),
            bracketed_paste: terminal.modes.get(Mode::BracketedPaste),
        })
    }

    /// Whether the frame describes an epoch this replica has left.
    fn stale(&self, epoch: Epoch) -> bool {
        self.epoch.is_some_and(|installed| epoch.0 < installed.0)
    }

    fn hold_geometry(&mut self, frame: &TerminalFrame) -> Applied {
        if !self.promised.geometry || self.stale(frame.epoch) {
            return Applied::default();
        }
        // An unreadable geometry is held as nothing: the snapshot it precedes
        // then has no geometry and is a desync, which is the honest answer.
        self.held_geometry = Geometry::decode(&frame.payload).map(|geometry| Held {
            epoch: frame.epoch,
            sequence: frame.sequence,
            value: geometry,
        });
        Applied::default()
    }

    fn hold_palette(&mut self, frame: &TerminalFrame) -> Applied {
        if !self.promised.palette || self.stale(frame.epoch) {
            return Applied::default();
        }
        self.held_palette = Some(Held {
            epoch: frame.epoch,
            sequence: frame.sequence,
            value: frame.payload.clone(),
        });
        Applied::default()
    }

    fn install(&mut self, frame: &TerminalFrame) -> Applied {
        if self.stale(frame.epoch) {
            return Applied::default();
        }
        let geometry = if self.promised.geometry {
            // The prefix members are one state point with the snapshot they
            // precede (ADR 0017 D1, D4). A snapshot without its geometry is
            // not installed as authoritative; the stream is desynchronised.
            match self.held_geometry.take() {
                Some(held) if held.describes(frame) => held.value,
                _ => {
                    self.held_palette = None;
                    return self.recover();
                }
            }
        } else {
            // Legacy: the size this client last asked for, and nothing before
            // one exists (Q13). The snapshot that arrived was built for nobody.
            match self.requested {
                Some(requested) => requested,
                None => {
                    self.screen = Err(Absence::AwaitingGrid);
                    return Applied::default();
                }
            }
        };
        let palette = match self.held_palette.take() {
            Some(held) if held.describes(frame) => Some(held.value),
            // A checkpoint stamped for another state point is an internally
            // inconsistent bundle, not installed (ADR 0017 D4).
            Some(_) => return self.recover(),
            None => None,
        };

        let terminal = Terminal::new(Options {
            cols: geometry.cols,
            rows: geometry.rows,
            ..Options::default()
        });
        let mut stream = Stream::new(TerminalHandler::new(terminal));
        let fed = catch_unwind(AssertUnwindSafe(|| {
            if let Some(palette) = &palette {
                feed(&mut stream, palette);
            }
            feed(&mut stream, &frame.payload);
        }));
        if fed.is_err() {
            return self.poisoned();
        }

        if self.epoch != Some(frame.epoch) {
            // A new epoch. The daemon's own — another viewer's resize, a
            // reshape — starts a fresh recovery episode; one this client asked
            // for does not, or a failing replica could resize its way to an
            // unbounded retry (round 1, #4).
            if self.own_reshape_pending {
                self.own_reshape_pending = false;
            } else {
                self.recovery_spent = false;
            }
        }
        self.epoch = Some(frame.epoch);
        self.screen = Ok(Screen { stream, geometry });
        Applied {
            redraw: true,
            ..Applied::default()
        }
    }

    fn feed_delta(&mut self, frame: &TerminalFrame) -> Applied {
        let Some(installed) = self.epoch else {
            // Nothing installed yet: the prefix for this epoch is still coming,
            // or under legacy the grid is. Bytes for a screen that does not
            // exist are dropped, and the snapshot that will exist supersedes
            // them.
            return Applied::default();
        };
        if frame.epoch != installed {
            // Older is stale. Newer means this epoch's prefix never arrived:
            // the stream is desynchronised.
            if frame.epoch.0 > installed.0 {
                return self.recover();
            }
            return Applied::default();
        }
        let Ok(screen) = &mut self.screen else {
            return Applied::default();
        };
        match catch_unwind(AssertUnwindSafe(|| {
            feed(&mut screen.stream, &frame.payload)
        })) {
            Ok(()) => Applied {
                redraw: true,
                ..Applied::default()
            },
            Err(_) => self.poisoned(),
        }
    }

    /// The parser failed: the replica is destroyed, never the process, and
    /// one automatic recovery is attempted per episode (spike grill Q3).
    fn poisoned(&mut self) -> Applied {
        self.screen = Err(Absence::Unavailable);
        let mut applied = self.recover();
        applied.redraw = true;
        applied
    }

    /// Ask for a fresh screen once per episode. A second failure in the same
    /// episode stops automatic retry: what is on display is not trusted and
    /// is not shown as if it were.
    fn recover(&mut self) -> Applied {
        if self.recovery_spent {
            self.screen = Err(Absence::Unavailable);
            return Applied {
                redraw: true,
                ..Applied::default()
            };
        }
        self.recovery_spent = true;
        Applied {
            resync: true,
            ..Applied::default()
        }
    }
}

/// Feed bytes to the emulator and discard what it would answer: the daemon's
/// authoritative terminal answers the program's queries, not a replica.
fn feed(stream: &mut Stream<TerminalHandler>, bytes: &[u8]) {
    stream.feed(bytes);
    let _ = stream.handler.take_output();
}

#[cfg(test)]
#[path = "replica_tests.rs"]
mod tests;
