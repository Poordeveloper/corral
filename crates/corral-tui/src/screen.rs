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
}

/// A whole screen, composed before any of it is written.
///
/// One write per redraw: a screen painted in pieces tears, and this one is
/// repainted every second.
pub struct Frame {
    geometry: Geometry,
    drawn: u16,
    bytes: Vec<u8>,
}

impl FullScreen {
    pub fn take() -> std::io::Result<Self> {
        let mut out = std::io::stdout();
        out.write_all(TAKE)?;
        out.flush()?;
        Ok(Self { out })
    }

    /// This terminal's size right now.
    ///
    /// Read per frame rather than remembered: a person who resizes their
    /// window while the list is up gets a list that fits it, without this
    /// surface needing to hear about the resize.
    pub fn geometry(&self) -> Option<Geometry> {
        Geometry::of(&std::io::stdin())
    }

    pub fn show(&mut self, frame: Frame) -> std::io::Result<()> {
        self.out.write_all(&frame.bytes)?;
        self.out.flush()
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
            bytes,
        }
    }

    /// How many rows are still free below what has been drawn.
    pub fn remaining(&self) -> u16 {
        self.geometry.rows.saturating_sub(self.drawn)
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
        self.bytes
            .extend_from_slice(self.truncated(text).as_bytes());
        self.bytes.extend_from_slice(SHOW_CURSOR.as_bytes());
        self.drawn += 1;
    }

    fn write_line(&mut self, emphasis: Emphasis, text: &str) {
        let opening = match emphasis {
            Emphasis::Plain => PLAIN,
            Emphasis::Selected => INVERSE,
            Emphasis::Secondary => DIM,
        };
        self.bytes.extend_from_slice(opening.as_bytes());
        self.bytes
            .extend_from_slice(self.truncated(text).as_bytes());
        self.bytes.extend_from_slice(PLAIN.as_bytes());
        // Carriage return as well as newline: the terminal is in raw mode, so
        // a newline alone moves down a row and leaves the column where it was.
        self.bytes.extend_from_slice(b"\r\n");
        self.drawn += 1;
    }

    /// The text this terminal has room for, cut on a character boundary.
    ///
    /// Counted in characters rather than display columns: this surface renders
    /// program names and hexadecimal ids, so a double-width character can cost
    /// alignment but never correctness, and a width table is a dependency Q5's
    /// reasoning declines for the same reason it declined a framework.
    fn truncated(&self, text: &str) -> String {
        let room = usize::from(self.geometry.cols);
        match text.char_indices().nth(room) {
            Some((at, _)) => text[..at].to_owned(),
            None => text.to_owned(),
        }
    }
}

#[cfg(test)]
#[path = "screen_tests.rs"]
mod tests;
