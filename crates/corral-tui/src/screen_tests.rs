use super::*;

const SMALL: Geometry = Geometry { rows: 4, cols: 10 };

fn drawn(frame: Frame) -> String {
    String::from_utf8(frame.bytes).expect("the frame is text and escape bytes")
}

/// A frame never draws past the last row. One extra line scrolls the whole
/// screen up by one, which on a list repainted every second is a screen that
/// crawls.
#[test]
fn a_frame_stops_at_the_last_row() {
    let mut frame = Frame::new(SMALL);

    for _ in 0..10 {
        frame.line(Emphasis::Plain, "a row");
    }

    assert_eq!(frame.drawn, SMALL.rows);
    assert_eq!(frame.remaining(), 0);
    assert_eq!(drawn(frame).matches("a row").count(), 4);
}

/// Truncation cuts on a character boundary. Cutting a multi-byte character in
/// half would put a replacement character on the screen — or, for a terminal
/// reading bytes, nothing recognisable at all.
#[test]
fn a_line_wider_than_the_terminal_is_cut_between_characters() {
    let mut frame = Frame::new(SMALL);

    frame.line(Emphasis::Plain, "ééééééééééééé");

    let text = drawn(frame);
    assert!(text.contains(&"é".repeat(10)), "{text:?}");
    assert!(!text.contains(&"é".repeat(11)), "{text:?}");
}

/// Raw mode is on while the list is up, so a newline alone moves down without
/// returning to the first column and every row would start further right than
/// the last.
#[test]
fn every_line_returns_to_the_first_column() {
    let mut frame = Frame::new(SMALL);

    frame.line(Emphasis::Plain, "one");
    frame.line(Emphasis::Plain, "two");

    let text = drawn(frame);
    assert_eq!(text.matches("\r\n").count(), 2, "{text:?}");
}

/// The prompt is the last thing drawn and leaves the cursor in it, so a person
/// can see where what they type is going.
#[test]
fn the_prompt_shows_the_cursor_and_does_not_move_off_its_line() {
    let mut frame = Frame::new(SMALL);

    frame.prompt("run: sh");

    let text = drawn(frame);
    assert!(text.ends_with(SHOW_CURSOR), "{text:?}");
    assert!(!text.ends_with("\r\n"), "{text:?}");
}

/// Every redraw starts from a clean screen, and the cursor is hidden while the
/// list is being painted rather than skittering across it.
#[test]
fn a_frame_clears_before_it_draws() {
    let text = drawn(Frame::new(SMALL));

    assert!(text.starts_with(HIDE_CURSOR), "{text:?}");
    assert!(text.contains(HOME_AND_CLEAR), "{text:?}");
}
