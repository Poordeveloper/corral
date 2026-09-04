//! Minting a terminal snapshot: what a client is sent, and what it is told
//! about what it did not get.
//!
//! A snapshot is a claim about a screen, and a client that replays one must
//! arrive at the screen the daemon actually holds (ADR 0003). So the numbers
//! here are contract, not tuning: they decide what a person sees after every
//! attach and — because resync is the only recovery path — after every moment
//! a session was already in trouble.

use qwertty_term_vt::formatter::{Content, Format, FormatOpt, Options, TerminalExtra};
use qwertty_term_vt::modes::Mode;
use qwertty_term_vt::point::{Coordinate, Point, Tag};

use super::terminal::{AuthoritativeTerminal, Poisoned};

/// How much recent scrollback a snapshot tries to carry.
///
/// An experience target, not a guaranteed minimum: what a snapshot actually
/// carries is the smallest of retained history, this number, and what fits the
/// encoded budget (ADR 0003 D7). It covers rereading the last command's
/// output and finding a recent error — not browsing history, which is not this
/// phase's job.
pub const SNAPSHOT_SCROLLBACK_ROWS: usize = 2_000;

/// What a normal snapshot tries to cost on the wire.
///
/// Over it, the oldest scrollback is trimmed until it fits. Initial policy
/// default, not a wire constant (ADR 0003 D8).
pub const SNAPSHOT_TARGET_BYTES: usize = 1024 * 1024;

/// What no successful snapshot may exceed.
///
/// Not a second target: it exists for pathological geometry, extreme style
/// state, encoder explosion, and corrupted or malicious terminal state. A
/// viewport that alone encodes past it is refused rather than shipped in
/// pieces (ADR 0003 D8).
pub const SNAPSHOT_CEILING_BYTES: usize = 16 * 1024 * 1024;

/// The two numbers a snapshot is encoded against.
///
/// A type rather than two constants read at the call site, because the pair
/// only makes sense together: the target is what trimming aims at, the ceiling
/// is what nothing may pass, and a caller that could set one without the other
/// could invert them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotBudget {
    target_bytes: usize,
    ceiling_bytes: usize,
}

impl SnapshotBudget {
    /// The shipping policy defaults (ADR 0003 D8).
    pub const DEFAULT: Self = Self {
        target_bytes: SNAPSHOT_TARGET_BYTES,
        ceiling_bytes: SNAPSHOT_CEILING_BYTES,
    };

    /// A budget with different numbers, for exercising the degradation and
    /// refusal paths without minting a screen large enough to reach the real
    /// ceiling. The behaviour under test is the algorithm, not the constants.
    #[cfg(test)]
    pub(crate) fn of(target_bytes: usize, ceiling_bytes: usize) -> Self {
        Self {
            target_bytes,
            ceiling_bytes,
        }
    }
}

/// A snapshot, and the honest account of what it left out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    payload: Vec<u8>,
    included_scrollback_rows: usize,
    history_truncated_before: bool,
}

/// Why a screen could not be expressed as a snapshot.
///
/// One variant, deliberately: the only thing that stops a snapshot is a
/// viewport too large to send at all, and it is reported rather than papered
/// over because a client that received part of a viewport would render a
/// screen that never existed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotError {
    ViewportExceedsCeiling {
        encoded_bytes: usize,
        /// The ceiling that refused it.
        ///
        /// Carried rather than read from the module constant: the budget is a
        /// parameter, so a message quoting the default would state a limit the
        /// encoder did not apply.
        ceiling_bytes: usize,
    },
    /// The screen cannot be read at all: its parser panicked and left the
    /// structure half-modified. Refused rather than serialized, because
    /// reading it is unsound and a plausible-looking screen is worse than a
    /// stated absence.
    ScreenPoisoned(Poisoned),
}

impl Snapshot {
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// How many scrollback rows this snapshot carries.
    pub fn included_scrollback_rows(&self) -> usize {
        self.included_scrollback_rows
    }

    /// Whether history existed before what this snapshot carries.
    ///
    /// A statement about this snapshot, never a promise about what the daemon
    /// still holds or what some future request could fetch: a client may know
    /// history was omitted without being promised the omitted history remains
    /// retrievable (ADR 0003 D6).
    pub fn history_truncated_before(&self) -> bool {
        self.history_truncated_before
    }

    pub fn encoded_bytes(&self) -> usize {
        self.payload.len()
    }
}

/// Mint a snapshot of the terminal's current screen.
///
/// Degradation order is fixed: the oldest scrollback is sacrificed first, and
/// the current viewport is never traded away to meet the target — only the
/// ceiling can refuse it (ADR 0003 D8).
pub fn encode(terminal: &AuthoritativeTerminal) -> Result<Snapshot, SnapshotError> {
    encode_within(terminal, SnapshotBudget::DEFAULT)
}

/// `encode`, against an explicit budget.
pub fn encode_within(
    terminal: &AuthoritativeTerminal,
    budget: SnapshotBudget,
) -> Result<Snapshot, SnapshotError> {
    if let Some(poisoned) = terminal.poisoned() {
        return Err(SnapshotError::ScreenPoisoned(poisoned));
    }

    let available = retained_scrollback_rows(terminal);
    let mut rows = available.min(SNAPSHOT_SCROLLBACK_ROWS);

    let mut payload = render(terminal, rows);
    // Estimate, then verify. The row count that fits is proportional to the
    // bytes each row cost, so one estimate lands close; the halvings are for
    // screens whose cost is not uniform, and the loop is bounded because a
    // snapshot must not become a search.
    for _ in 0..8 {
        if payload.len() <= budget.target_bytes || rows == 0 {
            break;
        }
        let proportional = rows * budget.target_bytes / payload.len().max(1);
        rows = proportional.min(rows / 2);
        payload = render(terminal, rows);
    }

    if payload.len() > budget.target_bytes && rows > 0 {
        rows = 0;
        payload = render(terminal, 0);
    }

    // The viewport is what remains once every scrollback row is gone. If even
    // that is past the ceiling, there is no smaller honest answer: a client
    // sent half a viewport would render a screen that never existed.
    if rows == 0 && payload.len() > budget.ceiling_bytes {
        return Err(SnapshotError::ViewportExceedsCeiling {
            encoded_bytes: payload.len(),
            ceiling_bytes: budget.ceiling_bytes,
        });
    }

    Ok(Snapshot {
        payload,
        included_scrollback_rows: rows,
        history_truncated_before: rows < available,
    })
}

/// Rows of scrollback the emulator is holding above the active area.
///
/// Zero for a screen that may no longer be read; every caller has already
/// refused such a screen, and this keeps the arithmetic from being the place
/// that discovers it.
fn retained_scrollback_rows(terminal: &AuthoritativeTerminal) -> usize {
    let (Some(inner), Some(geometry)) = (terminal.terminal(), terminal.geometry()) else {
        return 0;
    };
    inner
        .screens
        .active()
        .pages
        .total_rows()
        .saturating_sub(usize::from(geometry.rows()))
}

/// Serialize the viewport plus `scrollback_rows` of history.
fn render(terminal: &AuthoritativeTerminal, scrollback_rows: usize) -> Vec<u8> {
    let (Some(inner), Some(geometry)) = (terminal.terminal(), terminal.geometry()) else {
        return Vec::new();
    };
    let total_rows = inner.screens.active().pages.total_rows();
    let viewport_rows = usize::from(geometry.rows());
    let first_row = total_rows.saturating_sub(viewport_rows + scrollback_rows);

    let content = match total_rows.checked_sub(1) {
        None => Content::None,
        Some(last_row) => Content::Range {
            tl: Point::new(Tag::Screen, Coordinate::new(0, first_row as u32)),
            br: Point::new(
                Tag::Screen,
                Coordinate::new(inner.cols.saturating_sub(1), last_row as u32),
            ),
        },
    };

    // The palette is not here: it belongs to the subscription, because resync
    // is the recovery path and 5 KB of unchanging colours would be paid again
    // at exactly the worst moment (ADR 0003 D4).
    let extra = TerminalExtra {
        palette: false,
        ..TerminalExtra::all()
    };

    let mut payload = inner
        .format_content(&Options::vt(), &extra, content)
        .into_bytes();

    // The formatter never emits trailing blank rows, so a client scrolls
    // fewer times than there are history rows and history lands on its
    // screen (PR9 spike S2). The formatter itself says how many rows it
    // emitted: the same range as plain text, one row per line, under the
    // same trailing-blank rule. Each missing row becomes an LF past the last
    // row, which is exactly one scroll, so the history rows end in history.
    // A second pass over a range the budget already bounds (D7); once per
    // attach, resync, or reflow.
    let range_rows = total_rows.saturating_sub(first_row);
    let emitted_rows = match content {
        Content::None => 0,
        _ => {
            let plain = inner.format_content(
                &Options {
                    emit: FormatOpt(Format::Plain),
                    unwrap: false,
                    trim: false,
                    ..Options::default()
                },
                &TerminalExtra::none(),
                content,
            );
            if plain.is_empty() {
                0
            } else {
                plain.matches('\n').count() + 1
            }
        }
    };
    // Everything that constrains movement is opened for the trailer and
    // restored after it: the scrolling region (a line feed scrolls into
    // history only at the bottom of a full-screen region), origin mode (so
    // positioning is absolute), and left/right margins (which also bend the
    // formatter's own tab-stop trailer: its `CHA`s are margin-relative, so
    // the stops are restated absolutely below when margins are on).
    let region = inner.scrolling_region;
    let origin_mode = inner.modes.get(Mode::Origin);
    let margins = inner.modes.get(Mode::EnableLeftAndRightMargin);
    payload.extend_from_slice(b"\x1b[?6l\x1b[r");
    if margins {
        payload.extend_from_slice(b"\x1b[?69l");
    }
    if emitted_rows < range_rows {
        // The extras have already moved the cursor, so the padding first
        // returns to the last row that has content: the last screen row when
        // the content overflowed, its own row otherwise.
        let last_content_row = emitted_rows.clamp(1, viewport_rows);
        payload.extend_from_slice(format!("\x1b[{last_content_row};1H").as_bytes());
        for _ in emitted_rows..range_rows {
            payload.extend_from_slice(b"\r\n");
        }
    }
    if margins {
        payload.extend_from_slice(b"\x1b[3g");
        for col in (0..usize::from(inner.cols)).filter(|col| inner.tabstops.get(*col)) {
            payload.extend_from_slice(format!("\x1b[{}G\x1bH", col + 1).as_bytes());
        }
        payload.extend_from_slice(
            format!("\x1b[?69h\x1b[{};{}s", region.left + 1, region.right + 1).as_bytes(),
        );
    }
    if region.top != 0 || usize::from(region.bottom) + 1 != viewport_rows {
        payload.extend_from_slice(
            format!("\x1b[{};{}r", region.top + 1, region.bottom + 1).as_bytes(),
        );
    }
    if origin_mode {
        payload.extend_from_slice(b"\x1b[?6h");
    }

    if let Some(title) = terminal.title() {
        payload.extend_from_slice(b"\x1b]2;");
        payload.extend_from_slice(title);
        payload.push(0x07);
    }

    // The formatter writes the cursor position before its tab-stop trailer,
    // whose `CHA`s move the cursor and are never undone, so every client
    // began on the last tab stop (PR9 spike S1). Corral restates position and
    // visibility last, after everything that moves the cursor — the padding
    // above included. The formatter is compensated for, not claimed fixed.
    // Origin mode makes CUP relative to the region's corner, and the extras
    // restored that mode, so the restated position is relative when it is on.
    let cursor = &inner.screens.active().cursor;
    let (row, col) = if origin_mode {
        let left = if inner.modes.get(Mode::EnableLeftAndRightMargin) {
            region.left
        } else {
            0
        };
        (cursor.y - region.top, cursor.x - left)
    } else {
        (cursor.y, cursor.x)
    };
    payload.extend_from_slice(format!("\x1b[{};{}H", row + 1, col + 1).as_bytes());
    payload.extend_from_slice(if inner.modes.get(Mode::CursorVisible) {
        b"\x1b[?25h"
    } else {
        b"\x1b[?25l"
    });

    payload
}

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ViewportExceedsCeiling {
                encoded_bytes,
                ceiling_bytes,
            } => write!(
                f,
                "the viewport alone encodes to {encoded_bytes} bytes, past the {ceiling_bytes}-byte ceiling"
            ),
            Self::ScreenPoisoned(Poisoned::ParserPanicked) => f.write_str(
                "this terminal's parser failed on provider output and its screen can no longer be read",
            ),
        }
    }
}

impl std::error::Error for SnapshotError {}

#[cfg(test)]
#[path = "snapshot_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "snapshot_fidelity_tests.rs"]
mod fidelity_tests;
