//! The effective palette as a checkpoint a connection can be brought to.
//!
//! ADR 0003 D4 keeps the palette out of the snapshot; ADR 0017 D3 carries it
//! on its own frame, sent only when the connection's last checkpoint differs
//! from the screen's effective palette — including a return to the default,
//! which is why the payload begins with a reset: a resync must not depend on
//! a reset delta the client may never have received.

use qwertty_term_vt::color::{DEFAULT, Palette, Rgb};
use qwertty_term_vt::terminal::Terminal;

/// The palette state a snapshot point has, compared semantically.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaletteCheckpoint {
    palette: Palette,
    foreground: Option<Rgb>,
    background: Option<Rgb>,
}

impl PaletteCheckpoint {
    /// The baseline every connection starts from: a replica that has been
    /// told nothing renders with the built-in palette.
    pub const BASELINE: Self = Self {
        palette: DEFAULT,
        foreground: None,
        background: None,
    };

    /// The screen's effective palette now.
    pub fn of(terminal: &Terminal) -> Self {
        // The viewport window carries the live colour state without cloning
        // history, which is all this needs.
        let window = terminal.snapshot_window(0);
        Self {
            palette: window.palette,
            foreground: window.default_fg,
            background: window.default_bg,
        }
    }

    /// The frame a connection at `known` needs to reach `self`, if any.
    pub fn frame_from(&self, known: &Self) -> Option<Vec<u8>> {
        (self != known).then(|| self.payload())
    }

    /// The checkpoint as the OSC sequences a replica applies: a reset of
    /// every colour first, then each entry that is not the default, then the
    /// dynamic foreground and background when set. Exact whatever the client
    /// held before, which is the point of a checkpoint.
    fn payload(&self) -> Vec<u8> {
        let mut out = String::from("\x1b]104\x07\x1b]110\x07\x1b]111\x07");
        for (index, colour) in self.palette.iter().enumerate() {
            if *colour != DEFAULT[index] {
                out.push_str(&format!(
                    "\x1b]4;{index};rgb:{:02x}/{:02x}/{:02x}\x07",
                    colour.r, colour.g, colour.b
                ));
            }
        }
        if let Some(fg) = self.foreground {
            out.push_str(&format!(
                "\x1b]10;rgb:{:02x}/{:02x}/{:02x}\x07",
                fg.r, fg.g, fg.b
            ));
        }
        if let Some(bg) = self.background {
            out.push_str(&format!(
                "\x1b]11;rgb:{:02x}/{:02x}/{:02x}\x07",
                bg.r, bg.g, bg.b
            ));
        }
        out.into_bytes()
    }
}

#[cfg(test)]
#[path = "palette_tests.rs"]
mod tests;
