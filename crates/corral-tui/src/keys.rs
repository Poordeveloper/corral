//! What the person's bytes mean to a list.
//!
//! Small on purpose: enough to move a cursor, choose a row, type a command and
//! leave. Anything richer — key maps, chords, a mode system — is the framework
//! Q5 declined.
//!
//! An attached session never comes through here. It gets the bytes undecoded,
//! because the person's own terminal is the replica and Corral has no business
//! interpreting what they typed at their agent (`ARCHITECTURE.md` §3).

/// One key the list acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Enter,
    Escape,
    Backspace,
    /// `Ctrl-C`. Distinct from Escape: one leaves the list, the other leaves
    /// whatever the person was typing.
    Interrupt,
    /// A character the person typed.
    Typed(char),
    /// Something this build has no meaning for. Kept as a value rather than
    /// dropped, so an escape sequence can be consumed whole instead of its
    /// insides arriving as typed characters.
    Unknown,
}

/// The escape-sequence introducers a cursor key arrives under.
///
/// Both, because a terminal in application cursor mode sends `ESC O A` where
/// the same key sends `ESC [ A` otherwise, and a session the person just
/// detached from may well have left the mode set.
const CSI: u8 = b'[';
const SS3: u8 = b'O';

/// Decode a burst of typing into the keys it carries.
pub fn decode(bytes: &[u8]) -> Vec<Key> {
    let mut keys = Vec::new();
    let mut at = 0;

    while at < bytes.len() {
        let byte = bytes[at];
        match byte {
            0x1b => {
                let (key, consumed) = escape_sequence(&bytes[at..]);
                keys.push(key);
                at += consumed;
            }
            b'\r' | b'\n' => {
                keys.push(Key::Enter);
                at += 1;
            }
            // Both, because which one a terminal sends for its backspace key
            // is a local setting rather than a fact about the key.
            0x7f | 0x08 => {
                keys.push(Key::Backspace);
                at += 1;
            }
            0x03 => {
                keys.push(Key::Interrupt);
                at += 1;
            }
            // `Ctrl-N` and `Ctrl-P`, so the muscle memory of every other list
            // in a terminal works here too.
            0x0e => {
                keys.push(Key::Down);
                at += 1;
            }
            0x10 => {
                keys.push(Key::Up);
                at += 1;
            }
            _ if byte.is_ascii_control() => {
                keys.push(Key::Unknown);
                at += 1;
            }
            _ => {
                let (key, consumed) = character(&bytes[at..]);
                keys.push(key);
                at += consumed;
            }
        }
    }

    keys
}

/// One escape sequence, and how many bytes it took.
///
/// A lone escape is the Escape key: it is what a person pressing it produces,
/// and waiting for a continuation that will never come would swallow it.
fn escape_sequence(bytes: &[u8]) -> (Key, usize) {
    match bytes.get(1) {
        Some(&CSI) | Some(&SS3) => {
            // Parameters and intermediates run until the byte that ends the
            // sequence; anything unterminated in this burst is consumed whole
            // rather than leaking out as typed characters.
            let end = bytes[2..]
                .iter()
                .position(|byte| (0x40..=0x7e).contains(byte));
            match end {
                Some(offset) => {
                    let key = match bytes[2 + offset] {
                        b'A' => Key::Up,
                        b'B' => Key::Down,
                        _ => Key::Unknown,
                    };
                    (key, 3 + offset)
                }
                None => (Key::Unknown, bytes.len()),
            }
        }
        _ => (Key::Escape, 1),
    }
}

/// One character, and how many bytes it took.
///
/// A multi-byte character split across two reads loses its first half here.
/// Accepted rather than buffered: the one place characters are typed is the
/// prompt for a command to run, and carrying a decoder's state across reads
/// would be machinery for a case a person fixes with backspace.
fn character(bytes: &[u8]) -> (Key, usize) {
    let width = utf8_width(bytes[0]);
    let taken = width.min(bytes.len());
    match std::str::from_utf8(&bytes[..taken])
        .ok()
        .and_then(|text| text.chars().next())
    {
        Some(character) => (Key::Typed(character), taken),
        None => (Key::Unknown, 1),
    }
}

/// How many bytes the character starting with this one occupies.
fn utf8_width(lead: u8) -> usize {
    match lead {
        0x00..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf7 => 4,
        // A continuation byte with no lead, which is not a character start.
        _ => 1,
    }
}

#[cfg(test)]
#[path = "keys_tests.rs"]
mod tests;
