//! What a keystroke means to the session, decided from the replica's modes.
//!
//! The wire stays dumb: an `Input` frame carries bytes, and what bytes a key
//! produces is a fact about the terminal the program believes it is talking
//! to — which is the replica, in the modes the daemon's stream put it in
//! (PR9 plan, D2). The set is the one round 2 Q9 ruled: printable text,
//! Enter, Backspace, Tab, Escape, the cursor keys under DECCKM, Ctrl with a
//! letter, paste under bracketed paste, and Interrupt. No mouse reporting in
//! any encoding, and no claim of every keyboard protocol.

use crate::replica::Modes;

/// The accepted terminal representation of Ctrl-C: ETX, as an `Input` frame.
/// Not a function of the replica's modes (round 1).
pub const INTERRUPT: &[u8] = b"\x03";

/// One key press, in the vocabulary the window reports it in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyPress<'a> {
    /// The key's name: a letter, a digit, `enter`, `up`, `space`, …
    pub key: &'a str,
    /// The character this press would have typed, when it types one.
    pub typed: Option<&'a str>,
    pub control: bool,
    pub alt: bool,
    pub shift: bool,
    /// The platform key (Command on macOS): the application's own shortcuts,
    /// never the session's.
    pub platform: bool,
}

/// The bytes a key press sends, or `None` for a press that means nothing to
/// the session.
#[must_use]
pub fn encode(press: &KeyPress<'_>, modes: Modes) -> Option<Vec<u8>> {
    if press.platform {
        return None;
    }
    let cursor = |letter: u8| {
        if modes.cursor_keys {
            vec![0x1b, b'O', letter]
        } else {
            vec![0x1b, b'[', letter]
        }
    };
    let bytes = match press.key {
        "enter" => b"\r".to_vec(),
        "backspace" => b"\x7f".to_vec(),
        "tab" if press.shift => b"\x1b[Z".to_vec(),
        "tab" => b"\t".to_vec(),
        "escape" => b"\x1b".to_vec(),
        "up" => cursor(b'A'),
        "down" => cursor(b'B'),
        "right" => cursor(b'C'),
        "left" => cursor(b'D'),
        "home" => cursor(b'H'),
        "end" => cursor(b'F'),
        "delete" => b"\x1b[3~".to_vec(),
        "pageup" => b"\x1b[5~".to_vec(),
        "pagedown" => b"\x1b[6~".to_vec(),
        "space" if press.control => vec![0],
        key if press.control => vec![control(key)?],
        _ => press.typed?.as_bytes().to_vec(),
    };
    Some(bytes)
}

/// The control character a key produces with Ctrl held: letters, and the
/// four ASCII punctuation marks that share the range.
fn control(key: &str) -> Option<u8> {
    let mut characters = key.chars();
    let (first, rest) = (characters.next()?, characters.next());
    if rest.is_some() {
        return None;
    }
    match first.to_ascii_lowercase() {
        letter @ 'a'..='z' => Some(letter as u8 - b'a' + 1),
        '[' => Some(0x1b),
        '\\' => Some(0x1c),
        ']' => Some(0x1d),
        '^' => Some(0x1e),
        '_' => Some(0x1f),
        _ => None,
    }
}

/// Pasted text as the session receives it.
///
/// Wrapped when the program asked for bracketed paste, so it can tell a paste
/// from typing; otherwise line ends become carriage returns, which is what
/// typing them would send.
#[must_use]
pub fn paste(text: &str, modes: Modes) -> Vec<u8> {
    if modes.bracketed_paste {
        [b"\x1b[200~".as_slice(), text.as_bytes(), b"\x1b[201~"].concat()
    } else {
        text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
    }
}

#[cfg(test)]
#[path = "input_tests.rs"]
mod tests;
