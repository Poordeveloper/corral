use super::*;

fn press(key: &str) -> KeyPress<'_> {
    KeyPress {
        key,
        typed: None,
        control: false,
        alt: false,
        shift: false,
        platform: false,
    }
}

const NORMAL: Modes = Modes {
    cursor_keys: false,
    bracketed_paste: false,
};
const APPLICATION: Modes = Modes {
    cursor_keys: true,
    bracketed_paste: true,
};

#[test]
fn the_ruled_keys_send_their_bytes() {
    assert_eq!(encode(&press("enter"), NORMAL), Some(b"\r".to_vec()));
    assert_eq!(encode(&press("backspace"), NORMAL), Some(b"\x7f".to_vec()));
    assert_eq!(encode(&press("tab"), NORMAL), Some(b"\t".to_vec()));
    assert_eq!(encode(&press("escape"), NORMAL), Some(b"\x1b".to_vec()));
    assert_eq!(
        encode(
            &KeyPress {
                shift: true,
                ..press("tab")
            },
            NORMAL
        ),
        Some(b"\x1b[Z".to_vec())
    );
}

/// DECCKM is the replica's to report: the same key sends `ESC [ A` to a
/// shell and `ESC O A` to a full-screen program that switched the mode.
#[test]
fn cursor_keys_follow_the_replicas_mode() {
    assert_eq!(encode(&press("up"), NORMAL), Some(b"\x1b[A".to_vec()));
    assert_eq!(encode(&press("up"), APPLICATION), Some(b"\x1bOA".to_vec()));
    assert_eq!(encode(&press("left"), NORMAL), Some(b"\x1b[D".to_vec()));
    assert_eq!(encode(&press("end"), APPLICATION), Some(b"\x1bOF".to_vec()));
}

#[test]
fn control_with_a_letter_is_the_control_character() {
    let ctrl = |key| KeyPress {
        control: true,
        ..press(key)
    };
    assert_eq!(encode(&ctrl("c"), NORMAL), Some(vec![3]));
    assert_eq!(encode(&ctrl("z"), NORMAL), Some(vec![26]));
    assert_eq!(encode(&ctrl("["), NORMAL), Some(vec![0x1b]));
    assert_eq!(encode(&ctrl("space"), NORMAL), Some(vec![0]));
    assert_eq!(encode(&ctrl("1"), NORMAL), None);
}

#[test]
fn printable_text_is_what_the_key_typed_and_the_platform_key_is_not_the_sessions() {
    let typed = KeyPress {
        typed: Some("S"),
        shift: true,
        ..press("s")
    };
    assert_eq!(encode(&typed, NORMAL), Some(b"S".to_vec()));

    let shortcut = KeyPress {
        typed: Some("v"),
        platform: true,
        ..press("v")
    };
    assert_eq!(encode(&shortcut, NORMAL), None);

    // A modifier on its own types nothing.
    assert_eq!(encode(&press("shift"), NORMAL), None);
}

#[test]
fn a_paste_is_bracketed_when_asked_for_and_typed_when_not() {
    assert_eq!(
        paste("ls\nls\n", APPLICATION),
        b"\x1b[200~ls\nls\n\x1b[201~".to_vec()
    );
    assert_eq!(paste("ls\r\nls\n", NORMAL), b"ls\rls\r".to_vec());
}

#[test]
fn interrupt_is_etx() {
    assert_eq!(INTERRUPT, b"\x03");
}
