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

/// A decoder that survives a read boundary.
///
/// A burst is whatever one `read` returned, not whatever the person finished
/// typing: `ESC [ 1 ;` and `5 C` can arrive as two of them. Bytes that are not
/// a key yet are held here until the rest arrive, and bytes nobody acted on go
/// back to the person's terminal rather than being dropped.
#[derive(Default)]
pub struct Keyboard {
    held: Vec<u8>,
}

impl Keyboard {
    /// Add what one read produced to whatever was left over from the last.
    pub fn add(&mut self, bytes: &[u8]) {
        self.held.extend_from_slice(bytes);
    }

    /// The next key, or `None` while what is held is not a key yet.
    pub fn next(&mut self) -> Option<Key> {
        if self.undecided() {
            return None;
        }
        let (key, consumed) = decode_one(&self.held)?;
        self.held.drain(..consumed);
        Some(key)
    }

    /// Whether what is held is waiting on bytes that may never come.
    ///
    /// Two shapes. A bare `ESC` is the ambiguity a decoder cannot settle from
    /// bytes alone — it is both the Escape key and the first byte of every
    /// cursor key — and a local terminal writes a cursor key in one go where
    /// one behind ssh or tmux need not. Everything else here is a sequence or
    /// a character that arrived in part.
    ///
    /// Both wait, and both must be given up on: bytes held forever make the
    /// person's next key the missing final byte of a sequence they abandoned,
    /// which is a `q` that does not quit and says nothing.
    pub fn undecided(&self) -> bool {
        if self.held.is_empty() {
            return false;
        }
        self.held == [ESCAPE] || decode_one(&self.held).is_none()
    }

    /// Nothing followed, so what is held is all it will ever be.
    ///
    /// A bare Escape was the Escape key. Anything else never became one and is
    /// dropped rather than kept — reported, because a keystroke that did
    /// nothing should still redraw rather than look ignored.
    pub fn settle(&mut self) -> Option<Key> {
        if !self.undecided() {
            return None;
        }
        let escape = self.held == [ESCAPE];
        self.held.clear();
        Some(if escape { Key::Escape } else { Key::Unknown })
    }

    /// Everything read but not turned into a key.
    ///
    /// Taken when the terminal is handed to a session: what the person typed
    /// after the key that opened it was typed for what they opened, and a
    /// surface that has stopped reading has no claim on it.
    pub fn unread(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.held)
    }
}

/// The byte that begins every sequence, and is also a key.
const ESCAPE: u8 = 0x1b;

/// The escape-sequence introducers a cursor key arrives under.
///
/// Both, because a terminal in application cursor mode sends `ESC O A` where
/// the same key sends `ESC [ A` otherwise, and a session the person just
/// detached from may well have left the mode set.
const CSI: u8 = b'[';
const SS3: u8 = b'O';

/// The bytes a sequence may carry between its introducer and its end:
/// parameters and intermediates.
const PARAMETERS: std::ops::RangeInclusive<u8> = 0x20..=0x3f;
/// The byte that ends a sequence and says which one it was.
const FINAL: std::ops::RangeInclusive<u8> = 0x40..=0x7e;

/// Every key in one burst, for tests with no read boundary to model.
///
/// Through `Keyboard`, so what these assert is what the surface runs.
#[cfg(test)]
pub(crate) fn decode(bytes: &[u8]) -> Vec<Key> {
    let mut keyboard = Keyboard::default();
    keyboard.add(bytes);
    let mut keys: Vec<Key> = std::iter::from_fn(|| keyboard.next()).collect();
    // Nothing follows a burst a test hands over whole.
    keys.extend(keyboard.settle());
    keys
}

/// The next key, and how many bytes it took.
///
/// `None` when these bytes end in the middle of something that is not a key
/// yet — a `CSI` sequence with no final byte, a character with only some of
/// its bytes. A read boundary can fall anywhere, and deciding what a half of
/// something means is how `ESC [ 1 ;` becomes the characters `5` and `C` in
/// the command a person is about to run.
pub fn decode_one(bytes: &[u8]) -> Option<(Key, usize)> {
    let byte = *bytes.first()?;

    match byte {
        ESCAPE => escape_sequence(bytes),
        b'\r' | b'\n' => Some((Key::Enter, 1)),
        // Both, because which one a terminal sends for its backspace key is a
        // local setting rather than a fact about the key.
        0x7f | 0x08 => Some((Key::Backspace, 1)),
        0x03 => Some((Key::Interrupt, 1)),
        // `Ctrl-N` and `Ctrl-P`, so the muscle memory of every other list in a
        // terminal works here too.
        0x0e => Some((Key::Down, 1)),
        0x10 => Some((Key::Up, 1)),
        _ if byte.is_ascii_control() => Some((Key::Unknown, 1)),
        _ => character(bytes),
    }
}

/// One escape sequence, and how many bytes it took.
///
/// A lone escape is the Escape key: it is what a person pressing it produces,
/// and waiting for a continuation that will never come would swallow it. An
/// introducer with nothing after it yet is the other case — that one is a
/// sequence still arriving, and it waits.
fn escape_sequence(bytes: &[u8]) -> Option<(Key, usize)> {
    match bytes.get(1) {
        Some(&CSI) | Some(&SS3) => {
            for (offset, byte) in bytes[2..].iter().enumerate() {
                // Parameters and intermediates, which a sequence may carry any
                // number of before the byte that ends it.
                if PARAMETERS.contains(byte) {
                    continue;
                }
                if FINAL.contains(byte) {
                    let key = match byte {
                        b'A' => Key::Up,
                        b'B' => Key::Down,
                        _ => Key::Unknown,
                    };
                    return Some((key, 3 + offset));
                }
                // Anything else abandons the sequence, and what abandoned it
                // is a key: Ctrl-C among them, which raw mode has already
                // taken from the kernel, so this is the only path that
                // delivers it. Held as a parameter it would never arrive, and
                // a surface that cannot deliver Ctrl-C is one a person cannot
                // leave. Stop before it, and let it decode on its own.
                return Some((Key::Unknown, 2 + offset));
            }

            // Nothing but parameters so far: still arriving.
            None
        }
        // Escape on its own, and Escape at the end of a burst: a terminal
        // sends a cursor key's bytes in one write, so an introducer that is
        // not here yet is one that is not coming.
        _ => Some((Key::Escape, 1)),
    }
}

/// One character, and how many bytes it took.
///
/// A multi-byte character whose bytes have not all arrived is not a character
/// yet, and waits for the rest rather than being decoded from its first half.
fn character(bytes: &[u8]) -> Option<(Key, usize)> {
    let width = utf8_width(bytes[0]);
    if bytes.len() < width {
        return None;
    }

    match std::str::from_utf8(&bytes[..width])
        .ok()
        .and_then(|text| text.chars().next())
    {
        Some(character) => Some((Key::Typed(character), width)),
        // Not a character at all: a continuation byte with no lead, or a
        // sequence no width covers. One byte, so the rest still decodes.
        None => Some((Key::Unknown, 1)),
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
