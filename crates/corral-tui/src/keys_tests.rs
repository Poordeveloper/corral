use super::*;

#[test]
fn the_cursor_keys_arrive_under_both_introducers() {
    assert_eq!(decode(b"\x1b[A"), vec![Key::Up]);
    assert_eq!(decode(b"\x1b[B"), vec![Key::Down]);
    // What a terminal sends once a session left application cursor mode set.
    assert_eq!(decode(b"\x1bOA"), vec![Key::Up]);
    assert_eq!(decode(b"\x1bOB"), vec![Key::Down]);
}

/// An escape sequence is consumed whole. Leaking its insides would turn one
/// arrow key into a bracket and a letter, which in the prompt is two
/// characters a person did not type.
#[test]
fn a_sequence_this_build_has_no_meaning_for_is_consumed_whole() {
    assert_eq!(decode(b"\x1b[5~"), vec![Key::Unknown]);
    assert_eq!(decode(b"\x1b[1;5C"), vec![Key::Unknown]);
    assert_eq!(decode(b"\x1b[Ax"), vec![Key::Up, Key::Typed('x')]);
}

/// A sequence cut off by the end of a read is still not typed text. The rest
/// of it would arrive in the next burst, and rendering the half that came
/// first would put escape bytes in the prompt.
#[test]
fn an_unterminated_sequence_is_not_read_as_characters() {
    assert!(decode(b"\x1b[1;").is_empty());
    assert_eq!(decode(b"\x1b[1;5C"), vec![Key::Unknown]);
}

/// A read boundary can fall anywhere, including inside a cursor key. Decoding
/// what arrived would make `ESC [ 1 ;` and `5 C` into the characters `5` and
/// `C` — two the person never typed, in the command they are about to run.
#[test]
fn a_sequence_split_across_two_reads_is_still_one_key() {
    let mut keyboard = Keyboard::default();

    keyboard.add(b"\x1b[");
    assert_eq!(keyboard.next(), None);

    keyboard.add(b"B");
    assert_eq!(keyboard.next(), Some(Key::Down));
    assert_eq!(keyboard.next(), None);
}

/// The same for a character whose bytes are split.
#[test]
fn a_character_split_across_two_reads_is_still_one_key() {
    let mut keyboard = Keyboard::default();
    let bytes = "é".as_bytes();

    keyboard.add(&bytes[..1]);
    assert_eq!(keyboard.next(), None);

    keyboard.add(&bytes[1..]);
    assert_eq!(keyboard.next(), Some(Key::Typed('é')));
}

/// What the list did not act on is still the person's. A burst carrying the
/// key that opens a session and the first thing they meant to type into it
/// must not lose the second.
#[test]
fn what_was_read_but_not_acted_on_goes_back() {
    let mut keyboard = Keyboard::default();
    keyboard.add(b"\ryes\x1b[");

    assert_eq!(keyboard.next(), Some(Key::Enter));

    assert_eq!(keyboard.unread(), b"yes\x1b[");
    assert_eq!(keyboard.next(), None);
}

/// Escape on its own is the Escape key. Waiting for a continuation that never
/// comes would swallow the one key that cancels the prompt.
#[test]
fn a_lone_escape_is_the_escape_key() {
    assert_eq!(decode(b"\x1b"), vec![Key::Escape]);
    assert_eq!(decode(b"\x1bq"), vec![Key::Escape, Key::Typed('q')]);
}

#[test]
fn the_keys_a_list_acts_on_decode() {
    assert_eq!(decode(b"\r"), vec![Key::Enter]);
    assert_eq!(decode(b"\n"), vec![Key::Enter]);
    assert_eq!(decode(&[0x7f]), vec![Key::Backspace]);
    assert_eq!(decode(&[0x08]), vec![Key::Backspace]);
    assert_eq!(decode(&[0x03]), vec![Key::Interrupt]);
    assert_eq!(decode(&[0x0e]), vec![Key::Down]);
    assert_eq!(decode(&[0x10]), vec![Key::Up]);
}

#[test]
fn typing_decodes_a_character_at_a_time() {
    assert_eq!(decode(b"sh"), vec![Key::Typed('s'), Key::Typed('h')],);
}

/// A multi-byte character is one key, not one per byte: a prompt that took
/// three keys from one character would need three backspaces to remove it.
#[test]
fn a_multi_byte_character_is_one_key() {
    assert_eq!(decode("é".as_bytes()), vec![Key::Typed('é')]);
    assert_eq!(
        decode("→x".as_bytes()),
        vec![Key::Typed('→'), Key::Typed('x')]
    );
}

/// Every byte is either a key or the start of one, and no key costs zero
/// bytes: a decoder that could consume nothing would spin on the byte it did
/// not understand.
#[test]
fn every_byte_is_either_a_key_or_waiting_for_the_rest_of_one() {
    for byte in 0..=255_u8 {
        match decode_one(&[byte]) {
            Some((_, consumed)) => assert!(consumed >= 1, "{byte:#04x} consumed nothing"),
            None => assert!(
                utf8_width(byte) > 1,
                "{byte:#04x} decoded to nothing and was not the start of a character"
            ),
        }
    }
}

/// The escape hatch survives a sequence that never ends.
///
/// Raw mode has taken Ctrl-C from the kernel, so the decoder is the only thing
/// that delivers it. A held introducer that swallowed everything after it
/// would make the list unleavable from the terminal the person is at.
#[test]
fn an_abandoned_sequence_does_not_swallow_the_key_that_abandoned_it() {
    let mut keyboard = Keyboard::default();
    keyboard.add(b"\x1b[1;");

    assert_eq!(
        keyboard.next(),
        None,
        "an unfinished sequence decoded early"
    );

    keyboard.add(&[0x03]);
    assert_eq!(
        keyboard.next(),
        Some(Key::Unknown),
        "the abandoned sequence"
    );
    assert_eq!(keyboard.next(), Some(Key::Interrupt));
}
