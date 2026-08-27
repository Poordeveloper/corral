//! Corral-owned drawing for one full-screen list, and nothing more general.
//!
//! Q5 declined a TUI framework, and this module is the whole of what taking
//! that decision costs: rows, a highlighted one, dimmed secondary text, and a
//! frame written in one piece. Pane layout, widget composition and cell
//! rendering — the reasons to take a framework — are exactly what Q1 removed
//! by making Open a takeover. When several panes, scrollable structured views
//! or a modal system actually appear, that comparison happens again against
//! real requirements.

use std::io::Write;

use crate::attach::Geometry;

/// Take the whole terminal.
///
/// The alternate screen, so the person's own scrollback is still there when
/// Corral hands the terminal back — theirs from before they ran this, and not
/// Corral's to consume.
const TAKE: &[u8] = b"\x1b[?1049h";
/// Give it back, cursor and all.
const RELEASE: &[u8] = b"\x1b[?25h\x1b[?1049l";
/// Scrolling covers the whole screen again, whatever a session left set.
const WHOLE_SCREEN: &[u8] = b"\x1b[r";
/// No mouse reporting, in any of the ways a program turns it on.
///
/// A terminal still reporting sends `ESC [ M` and three coordinate bytes for
/// every click, and those bytes are indistinguishable from someone typing: a
/// click in the eighty-first column sends `q`. Nothing here reads a mouse, so
/// this is not a mode this surface can be left in.
const NO_MOUSE: &[u8] = b"\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l";

const HIDE_CURSOR: &str = "\x1b[?25l";
const SHOW_CURSOR: &str = "\x1b[?25h";
const HOME_AND_CLEAR: &str = "\x1b[H\x1b[2J";
const INVERSE: &str = "\x1b[7m";
const DIM: &str = "\x1b[2m";
const PLAIN: &str = "\x1b[0m";

/// How a line is emphasised. Three, because that is what a list needs: the row
/// under the cursor, the secondary text beneath a row, and everything else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Emphasis {
    Plain,
    Selected,
    Secondary,
}

/// The terminal, held for as long as the list is on it.
///
/// Owns the taking and the giving back, so every way out of the list — a
/// chosen session, a quit, an error on the way to either — leaves the person's
/// terminal as it was found.
pub struct FullScreen {
    out: std::io::Stdout,
    /// The last size this terminal reported.
    ///
    /// Kept because the question can go unanswered transiently — an ioctl
    /// interrupted by a signal answers nothing — and one unanswered question
    /// about the size is not a terminal that stopped being one.
    last: Option<Geometry>,
}

/// A whole screen, composed before any of it is written.
///
/// One write per redraw: a screen painted in pieces tears, and this one is
/// repainted every second.
pub struct Frame {
    geometry: Geometry,
    drawn: u16,
    /// Rows at the bottom the caller has spoken for and will draw last.
    reserved: u16,
    bytes: Vec<u8>,
}

impl FullScreen {
    pub fn take() -> std::io::Result<Self> {
        let mut screen = Self {
            out: std::io::stdout(),
            last: Geometry::of(&std::io::stdin()),
        };
        screen.claim()?;
        Ok(screen)
    }

    /// Put the terminal in the state this surface needs.
    ///
    /// Asserted rather than assumed, and in one place, because there are two
    /// moments it has to be true: taking the terminal, and taking it back from
    /// a session that drew whatever it liked on it.
    fn claim(&mut self) -> std::io::Result<()> {
        self.out.write_all(TAKE)?;
        self.out.write_all(WHOLE_SCREEN)?;
        self.out.write_all(NO_MOUSE)?;
        self.out.flush()
    }

    /// This terminal's size right now.
    ///
    /// Read per frame rather than remembered: a person who resizes their
    /// window while the list is up gets a list that fits it, without this
    /// surface needing to hear about the resize.
    pub fn geometry(&mut self) -> Option<Geometry> {
        if let Some(now) = Geometry::of(&std::io::stdin()) {
            self.last = Some(now);
        }
        self.last
    }

    pub fn show(&mut self, frame: Frame) -> std::io::Result<()> {
        self.out.write_all(&frame.bytes)?;
        self.out.flush()
    }

    /// Hand the terminal to whatever draws next, without giving the screen up.
    ///
    /// The alternate screen stays taken. Open is a takeover of *this* terminal
    /// (grill Q1), so the session paints here and the list is still underneath
    /// when the person comes back — releasing first would put the session's
    /// snapshot clear, and everything it drew after it, on the person's own
    /// screen. Only the cursor is handed over, because from here until they
    /// return it belongs to the session.
    pub fn hand_over(&mut self) -> std::io::Result<()> {
        self.out.write_all(SHOW_CURSOR.as_bytes())?;
        self.out.flush()
    }

    /// Take the terminal back once whatever it was handed to is done.
    ///
    /// A takeover replays a child's own bytes, so the terminal comes back in
    /// whatever state that child left: `vim` on exit writes `\x1b[?1049l` and
    /// drops out of the alternate screen, a full-screen program may leave a
    /// scroll region set, and one killed mid-run leaves mouse reporting on.
    /// None of that is something this surface can detect, and each of them
    /// breaks it differently — so what it needs is asserted again rather than
    /// assumed to have held.
    pub fn take_back(&mut self) -> std::io::Result<()> {
        self.claim()
    }
}

impl Drop for FullScreen {
    fn drop(&mut self) {
        // Best effort by necessity: a terminal that will not take these bytes
        // is one nothing here can restore, and failing loudly in a destructor
        // would replace a cosmetic problem with a lost error.
        let _ = self.out.write_all(RELEASE);
        let _ = self.out.flush();
    }
}

impl Frame {
    pub fn new(geometry: Geometry) -> Self {
        let mut bytes = Vec::with_capacity(usize::from(geometry.rows) * 64);
        bytes.extend_from_slice(HIDE_CURSOR.as_bytes());
        bytes.extend_from_slice(HOME_AND_CLEAR.as_bytes());
        Self {
            geometry,
            drawn: 0,
            reserved: 0,
            bytes,
        }
    }

    /// What this frame would write, for tests that assert about its text.
    #[cfg(test)]
    pub(crate) fn text(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.bytes)
    }

    /// Set aside rows at the bottom for lines that are drawn last.
    ///
    /// A footer and a prompt are written after the body but belong under it,
    /// so their rows have to be spoken for before the body starts: a body
    /// larger than the screen would otherwise take them, and the prompt is the
    /// only line that shows the cursor — losing it leaves a person typing
    /// blind into something they cannot see.
    pub fn reserve(&mut self, rows: u16) {
        self.reserved = rows;
    }

    /// How many rows are still free below what has been drawn, not counting
    /// what is reserved.
    pub fn remaining(&self) -> u16 {
        self.geometry
            .rows
            .saturating_sub(self.drawn.saturating_add(self.reserved))
    }

    /// Draw one line, truncated to the terminal's width.
    ///
    /// Silently ignored once the screen is full: a caller that has already
    /// consulted `remaining` is doing the layout, and a line past the last row
    /// would scroll the whole frame up by one.
    pub fn line(&mut self, emphasis: Emphasis, text: &str) {
        if self.remaining() == 0 {
            return;
        }
        self.write_line(emphasis, text);
    }

    /// Draw the line the person is typing on, and leave the cursor in it.
    ///
    /// The last thing a frame draws, because the cursor stays wherever the
    /// writing stopped — which is exactly after the last character typed.
    pub fn prompt(&mut self, text: &str) {
        if self.remaining() == 0 {
            return;
        }
        self.start_line();
        self.bytes
            .extend_from_slice(self.truncated(text).as_bytes());
        self.bytes.extend_from_slice(SHOW_CURSOR.as_bytes());
        self.drawn += 1;
    }

    /// Move to the start of the next row, for every line after the first.
    ///
    /// Before the line rather than after it, so a frame that fills the screen
    /// ends *on* the last row instead of one line past it: a newline written
    /// at the bottom margin scrolls the whole frame up by one, and on a screen
    /// repainted every second that means the top row is never seen.
    ///
    /// Carriage return as well as newline, because the terminal is in raw
    /// mode: a newline alone moves down a row and leaves the column where it
    /// was.
    fn start_line(&mut self) {
        if self.drawn > 0 {
            self.bytes.extend_from_slice(b"\r\n");
        }
    }

    fn write_line(&mut self, emphasis: Emphasis, text: &str) {
        let opening = match emphasis {
            Emphasis::Plain => PLAIN,
            Emphasis::Selected => INVERSE,
            Emphasis::Secondary => DIM,
        };
        self.start_line();
        self.bytes.extend_from_slice(opening.as_bytes());
        self.bytes
            .extend_from_slice(self.truncated(text).as_bytes());
        self.bytes.extend_from_slice(PLAIN.as_bytes());
        self.drawn += 1;
    }

    /// The text this terminal has room for, with nothing in it that moves the
    /// cursor.
    ///
    /// A row shows a title, which is the file name of a program somebody
    /// chose, and error text the daemon wrote. A Unix file name may contain a
    /// newline or an escape byte; drawn as it arrived it would advance rows
    /// this frame is not counting, or leave a colour the next line inherits.
    /// So a control character becomes the character that says a character
    /// could not be shown.
    ///
    /// Cut in characters rather than display columns: this surface renders
    /// program names and hexadecimal ids, so a double-width character can cost
    /// alignment but never correctness, and a width table is a dependency Q5's
    /// reasoning declines for the same reason it declined a framework.
    fn truncated(&self, text: &str) -> String {
        text.chars()
            .map(|character| {
                if character.is_control() {
                    char::REPLACEMENT_CHARACTER
                } else {
                    character
                }
            })
            .take(usize::from(self.geometry.cols))
            .collect()
    }
}

#[cfg(test)]
#[path = "screen_tests.rs"]
mod tests;
