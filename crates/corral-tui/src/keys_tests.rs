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
    assert_eq!(decode(b"\x1b[1;"), vec![Key::Unknown]);
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

/// Every byte is accounted for. A decoder that could consume nothing would
/// spin on the byte it did not understand.
#[test]
fn every_byte_produces_a_key_and_none_is_left_behind() {
    for byte in 0..=255_u8 {
        let keys = decode(&[byte]);

        assert_eq!(keys.len(), 1, "{byte:#04x} decoded to {keys:?}");
    }
}
